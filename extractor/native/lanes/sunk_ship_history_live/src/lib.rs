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
pub const SCHEMA_VERSION: u32 = 2;
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"466880cc290e8961dab48c8323346d9d862089d85bbe0f4ba285b924cc959c82");
pub const EXACT20_DATA_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"5e0ed3e76724032f3e2f4cc5bcc9e5c38347ed862e0489821be8b7bbdd45ca21");
pub const WRITER_SENTINEL_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"dccd61e7573317458980fc0796fcba2ba945a434fbacbf3303a588c620307a4a");
pub const ALL_WRITER_ROUTES_MANIFEST_SHA256: [u8; 32] =
    hex32(b"d02f0f1aaf2369db9dc9529062f5e8f59f3f6f751fb9e4c0ab81d47dea1e1de0");
pub const CSUNK_SHIP_VTABLE_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"61140fad7a24c150b60935580374129321719555bb5a4e1db88bd2bcd296937e");
pub const CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"755a0011376d06bbd38456a2bb009236d99f4481a9b076789d9d57729c5673fd");
pub const ATTACKER_GETTERS_MANIFEST_SHA256: [u8; 32] =
    hex32(b"946eff0fb077863ce2dc1f1607ac2c0b9fcfee8f3377662ff14315101c30cbd6");

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
    /// Exact victim country id serialized from CSunkShipInfo+0x48.
    pub victim_country_id_raw: u32,
    /// Exact attacker country id serialized from CSunkShipInfo+0x4c.
    pub attacker_country_id_raw: u32,
    /// Exact country id consumed by the item populator from CSunkShipInfo+0x78.
    pub ui_source_country_id_raw: u32,
    pub sink_date_raw: i32,
    /// Slot index copied from the outer, d49/d50-bound victim CFEXMember.
    /// This is not the unrelated CSunkShipInfo +0x84 reference handle.
    pub victim_member_battle_slot_index: u32,
    pub flags: u32,
    /// Pointer-free reference handles serialized from +0x7c and +0x84.
    pub victim_ship_reference_raw: [u32; 2],
    pub damage_source_reference_raw: [u32; 2],
    /// Exact optional task-force stable id serialized from the +0x90 owner.
    pub task_force_id_raw: u32,
    /// Exact pointer-free two-word value serialized from CSunkShipInfo+0x98.
    pub terminal_context_raw: [u32; 2],
    /// Exact serializer flag at CSunkShipInfo+0xA0; no broader semantics inferred.
    pub cause_record_flag: bool,
    pub ship_name: String,
    pub ship_class_name: String,
    /// Exact +0x28 text: attacker ship name or cause token, selected by +0xA0.
    pub attribution_source_name: String,
    pub attribution_source_class_name: String,
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
    let mut identities=BTreeSet::new(); let mut prior_sequence=0u64;
    for event in &snapshot.events {
        if event.event_sequence<=1 || event.combat_id==0 || event.combat_generation==0
            || event.victim_country_id_raw==0 || event.attacker_country_id_raw==0
            || event.sink_date_raw<=0
            || event.victim_member_battle_slot_index > naval_combat_live_lane::domain49_50_naval_combat_live::MAX_BATTLE_SLOT_INDEX
            || event.victim_ship_reference_raw == [0, 0]
            || event.flags & !EVENT_KNOWN_FLAGS != 0
            || (event.flags & EVENT_FLAG_KILLED != 0) == (event.flags & EVENT_FLAG_ASSISTED_KILL != 0)
            || (event.cause_record_flag && event.flags & EVENT_FLAG_ASSISTED_KILL != 0)
            || !valid_text(&event.ship_name,true) || !valid_text(&event.ship_class_name,true)
            || !valid_text(&event.attribution_source_name,false) || !valid_text(&event.attribution_source_class_name,false)
            || event.event_sequence<=prior_sequence || !identities.insert(event.event_sequence)
        { return invalid("SHP1 event is incomplete, ambiguous, or duplicated"); }
        prior_sequence=event.event_sequence;
    }
    if snapshot.events.last().map_or(snapshot.source_sequence!=1, |event| snapshot.source_sequence!=event.event_sequence) { return invalid("SHP1 source sequence does not match the complete ordered history"); }
    Ok(())
}

pub fn validate_frame(frame: &SunkShipFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable if frame.status_flags != 0 && frame.snapshot.is_none() => Ok(()),
        AuthorityState::StaticExact20Receipt if frame.status_flags == 0 && frame.frame_hour>0 && frame.snapshot.is_some() => {
            let snapshot=frame.snapshot.as_ref().unwrap(); validate_snapshot(snapshot)?;
            if snapshot.events.iter().any(|event| {
                compact_contract::calendar::hour_index_from_raw(event.sink_date_raw)
                    .and_then(|hour| u64::try_from(hour).ok())
                    .map_or(true, |hour| hour > frame.frame_hour)
            }) { return invalid("SHP1 sink date is invalid or exceeds the frame hour"); }
            Ok(())
        },
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
    u64v(&mut out,frame.elapsed_wall_ms);u64v(&mut out,frame.frame_hour);out.extend_from_slice(&EXECUTABLE_SHA256);out.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);out.extend_from_slice(&EXACT20_DATA_ARTIFACT_SHA256);out.extend_from_slice(&WRITER_SENTINEL_ARTIFACT_SHA256);out.extend_from_slice(&ALL_WRITER_ROUTES_MANIFEST_SHA256);out.extend_from_slice(&CSUNK_SHIP_VTABLE_ARTIFACT_SHA256);out.extend_from_slice(&CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256);out.extend_from_slice(&ATTACKER_GETTERS_MANIFEST_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));out.extend_from_slice(&[0;3]);
    if let Some(snapshot)=&frame.snapshot { u64v(&mut out,snapshot.source_sequence);out.push(u8::from(snapshot.producer_complete));out.extend_from_slice(&[0;3]);u32v(&mut out,snapshot.events.len() as u32); for e in &snapshot.events {
        u64v(&mut out,e.event_sequence);u32v(&mut out,e.combat_id);u64v(&mut out,e.combat_generation);u32v(&mut out,e.victim_country_id_raw);u32v(&mut out,e.attacker_country_id_raw);u32v(&mut out,e.ui_source_country_id_raw);i32v(&mut out,e.sink_date_raw);u32v(&mut out,e.victim_member_battle_slot_index);u32v(&mut out,e.flags);u32v(&mut out,e.victim_ship_reference_raw[0]);u32v(&mut out,e.victim_ship_reference_raw[1]);u32v(&mut out,e.damage_source_reference_raw[0]);u32v(&mut out,e.damage_source_reference_raw[1]);u32v(&mut out,e.task_force_id_raw);u32v(&mut out,e.terminal_context_raw[0]);u32v(&mut out,e.terminal_context_raw[1]);out.push(u8::from(e.cause_record_flag));out.extend_from_slice(&[0;3]);textv(&mut out,&e.ship_name);textv(&mut out,&e.ship_class_name);textv(&mut out,&e.attribution_source_name);textv(&mut out,&e.attribution_source_class_name);
    }} Ok(out)
}

pub fn decode_payload(payload:&[u8])->Result<SunkShipFrame,CarrierError>{
    let mut r=Reader{payload,at:0}; if r.u32()?!=MAGIC||r.u32()?!=SCHEMA_VERSION{return invalid("SHP1 magic/schema mismatch")};
    let authority=match r.u32()?{0=>AuthorityState::Unavailable,1=>AuthorityState::StaticExact20Receipt,_=>return invalid("SHP1 authority unknown")};let status_flags=r.u32()?;let elapsed_wall_ms=r.u64()?;let frame_hour=r.u64()?;
    if r.bytes(32)?!=EXECUTABLE_SHA256||r.bytes(32)?!=EXACT20_FUNCTION_MANIFEST_SHA256||r.bytes(32)?!=EXACT20_DATA_ARTIFACT_SHA256||r.bytes(32)?!=WRITER_SENTINEL_ARTIFACT_SHA256||r.bytes(32)?!=ALL_WRITER_ROUTES_MANIFEST_SHA256||r.bytes(32)?!=CSUNK_SHIP_VTABLE_ARTIFACT_SHA256||r.bytes(32)?!=CSUNK_SHIP_VIRTUALS_MANIFEST_SHA256||r.bytes(32)?!=ATTACKER_GETTERS_MANIFEST_SHA256{return invalid("SHP1 receipt identity mismatch")};
    let has=r.u8()?;if has>1||r.bytes(3)?!=[0,0,0]{return invalid("SHP1 availability marker invalid")};
    let snapshot=if has==1{let source_sequence=r.u64()?;let producer_complete=r.u8()?!=0;if r.bytes(3)?!=[0,0,0]{return invalid("SHP1 completeness reserved bytes invalid")};let count=r.u32()? as usize;if count>MAX_EVENTS{return invalid("SHP1 event count exceeds bound")};let mut events=Vec::with_capacity(count);for _ in 0..count{let event_sequence=r.u64()?;let combat_id=r.u32()?;let combat_generation=r.u64()?;let victim_country_id_raw=r.u32()?;let attacker_country_id_raw=r.u32()?;let ui_source_country_id_raw=r.u32()?;let sink_date_raw=r.i32()?;let victim_member_battle_slot_index=r.u32()?;let flags=r.u32()?;let victim_ship_reference_raw=[r.u32()?,r.u32()?];let damage_source_reference_raw=[r.u32()?,r.u32()?];let task_force_id_raw=r.u32()?;let terminal_context_raw=[r.u32()?,r.u32()?];let cause_record_flag=r.u8()?!=0;if r.bytes(3)?!=[0,0,0]{return invalid("SHP1 event reserved bytes invalid")};events.push(SunkShipEvent{event_sequence,combat_id,combat_generation,victim_country_id_raw,attacker_country_id_raw,ui_source_country_id_raw,sink_date_raw,victim_member_battle_slot_index,flags,victim_ship_reference_raw,damage_source_reference_raw,task_force_id_raw,terminal_context_raw,cause_record_flag,ship_name:r.text()?,ship_class_name:r.text()?,attribution_source_name:r.text()?,attribution_source_class_name:r.text()?});}Some(SunkShipSnapshot{source_sequence,producer_complete,events})}else{None};
    if r.at!=payload.len(){return Err(CarrierError::TrailingBytes)};let frame=SunkShipFrame{elapsed_wall_ms,frame_hour,status_flags,authority,snapshot};validate_frame(&frame)?;Ok(frame)
}

fn u32v(out:&mut Vec<u8>,v:u32){out.extend_from_slice(&v.to_le_bytes())} fn i32v(out:&mut Vec<u8>,v:i32){out.extend_from_slice(&v.to_le_bytes())} fn u64v(out:&mut Vec<u8>,v:u64){out.extend_from_slice(&v.to_le_bytes())}
fn textv(out:&mut Vec<u8>,v:&str){u32v(out,v.len() as u32);out.extend_from_slice(v.as_bytes())}
struct Reader<'a>{payload:&'a[u8],at:usize} impl<'a> Reader<'a>{fn bytes(&mut self,n:usize)->Result<&'a[u8],CarrierError>{let end=self.at.checked_add(n).ok_or(CarrierError::Truncated)?;let b=self.payload.get(self.at..end).ok_or(CarrierError::Truncated)?;self.at=end;Ok(b)}fn u8(&mut self)->Result<u8,CarrierError>{Ok(self.bytes(1)?[0])}fn u32(&mut self)->Result<u32,CarrierError>{Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))}fn i32(&mut self)->Result<i32,CarrierError>{Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))}fn u64(&mut self)->Result<u64,CarrierError>{Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))}fn text(&mut self)->Result<String,CarrierError>{let n=self.u32()? as usize;if n>MAX_TEXT_BYTES{return invalid("SHP1 text exceeds bound")};String::from_utf8(self.bytes(n)?.to_vec()).map_err(|_|CarrierError::Invalid("SHP1 text is not UTF-8".into()))}fn opt_text(&mut self)->Result<Option<String>,CarrierError>{match self.u8()?{0=>Ok(None),1=>Ok(Some(self.text()?)),_=>invalid("SHP1 optional text marker invalid")}}}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> SunkShipFrame {
        SunkShipFrame {
            elapsed_wall_ms: 7,
            frame_hour: 16_959_360,
            status_flags: 0,
            authority: AuthorityState::StaticExact20Receipt,
            snapshot: Some(SunkShipSnapshot {
                source_sequence: 2,
                producer_complete: true,
                events: vec![SunkShipEvent {
                    event_sequence: 2,
                    combat_id: 4,
                    combat_generation: 2,
                    victim_country_id_raw: 3,
                    attacker_country_id_raw: 9,
                    ui_source_country_id_raw: 5,
                    sink_date_raw: compact_contract::calendar::DATE_EPOCH_CONST + 16_959_300,
                    victim_member_battle_slot_index: 7,
                    flags: EVENT_FLAG_KILLED,
                    victim_ship_reference_raw: [11, 12],
                    damage_source_reference_raw: [13, 14],
                    task_force_id_raw: 15,
                    terminal_context_raw: [16, 17],
                    cause_record_flag: false,
                    ship_name: "HMS Exact".into(),
                    ship_class_name: "Destroyer".into(),
                    attribution_source_name: "KMS Exact".into(),
                    attribution_source_class_name: "Cruiser".into(),
                }],
            }),
        }
    }

    #[test]
    fn roundtrip_preserves_exact_epoch_and_writer_fields() {
        let frame = frame();
        assert_eq!(decode_payload(&encode_payload(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn optional_attacker_text_and_damage_source_absence_are_preserved() {
        let mut frame = frame();
        let event = &mut frame.snapshot.as_mut().unwrap().events[0];
        event.damage_source_reference_raw = [0, 0];
        event.attribution_source_name.clear();
        event.attribution_source_class_name.clear();
        assert_eq!(decode_payload(&encode_payload(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn production_source_is_inert_until_fixture_promotion() {
        struct Source;
        impl ExactSunkShipSource for Source {
            fn is_exact_a729_writer_source(&self) -> bool { true }
            fn read_sunk_ship_snapshot(&self, _: u64) -> Result<SunkShipSnapshot, String> {
                Ok(frame().snapshot.unwrap())
            }
        }
        assert_eq!(collect_and_encode(&Source, 900, 0), Err(CarrierError::ActivationDisabled));
    }

    #[test]
    fn disappearance_style_ambiguous_flags_are_rejected() {
        let mut frame = frame();
        frame.snapshot.as_mut().unwrap().events[0].flags = 0;
        assert!(encode_payload(&frame).is_err());
    }

    #[test]
    fn date_slot_order_and_history_tail_are_fail_closed() {
        let mut future = frame();
        future.snapshot.as_mut().unwrap().events[0].sink_date_raw =
            compact_contract::calendar::DATE_EPOCH_CONST + 16_959_361;
        assert!(encode_payload(&future).is_err());

        let mut invalid_slot = frame();
        invalid_slot.snapshot.as_mut().unwrap().events[0].victim_member_battle_slot_index = 4_097;
        assert!(encode_payload(&invalid_slot).is_err());

        let mut stale_tail = frame();
        stale_tail.snapshot.as_mut().unwrap().source_sequence = 3;
        assert!(encode_payload(&stale_tail).is_err());

        let mut reordered = frame();
        let mut older = reordered.snapshot.as_ref().unwrap().events[0].clone();
        older.event_sequence = 1;
        reordered.snapshot.as_mut().unwrap().events[0].event_sequence = 2;
        reordered.snapshot.as_mut().unwrap().events.push(older);
        reordered.snapshot.as_mut().unwrap().source_sequence = 1;
        assert!(encode_payload(&reordered).is_err());
    }
}

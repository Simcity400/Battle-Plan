pub const DOMAIN_ID: u16 = 97;
pub const DOMAIN_NAME: &str = "peace_conference_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"PCS1");
pub const AUTHORITY_EXACT_A729_PEACE_CONFERENCE: u32 = 1;

pub const EXACT_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const PRODUCER_MANIFEST_SHA256: [u8; 32] = [
    0xe7, 0xd3, 0x06, 0xa8, 0x2c, 0xc8, 0x1b, 0x1f, 0x5c, 0x78, 0xda, 0xc6, 0x28, 0x8f, 0xc0, 0xc8,
    0x36, 0x6c, 0x11, 0xbc, 0xac, 0x5c, 0xfd, 0xa8, 0xac, 0x03, 0xbf, 0xb6, 0x61, 0xad, 0xdb, 0x30,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeaceConferenceReference {
    pub object_type: u32,
    pub object_id: u32,
}

impl PeaceConferenceReference {
    pub const fn valid(self) -> bool {
        self.object_type != 0 || self.object_id != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivePeaceConferenceObservation {
    pub refresh_sequence: u64,
    pub refresh_epoch: u64,
    pub conference_reference: PeaceConferenceReference,
    pub current_turn_raw: i32,
    pub completed: bool,
    pub semantic_snapshot_complete: bool,
}

pub const MEASURED_EMPTY_STATUS: u32 = compact_contract::COMPACT_STATUS_SOURCE_EMPTY;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeaceConferenceObservation {
    Inactive { refresh_sequence: u64 },
    Active(ActivePeaceConferenceObservation),
    NoConferenceInProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaceConferenceTransportFrame {
    pub frame_hour: u64,
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub observation: Option<PeaceConferenceObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    if status_flags == 0 {
        return invalid("PCS1 unavailable frame requires nonzero status");
    }
    if status_flags & MEASURED_EMPTY_STATUS != 0 {
        return invalid("PCS1 unavailable frame may not carry the source-empty bit");
    }
    encode_payload(&PeaceConferenceTransportFrame {
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        observation: None,
    })
}

pub fn encode_no_conference_in_progress(
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&PeaceConferenceTransportFrame {
        frame_hour,
        elapsed_wall_ms,
        status_flags: MEASURED_EMPTY_STATUS,
        observation: Some(PeaceConferenceObservation::NoConferenceInProgress),
    })
}

pub fn encode_payload(frame: &PeaceConferenceTransportFrame) -> Result<Vec<u8>, CarrierError> {
    validate(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_EXACT_A729_PEACE_CONFERENCE);
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.frame_hour);
    u64v(&mut out, frame.elapsed_wall_ms);
    out.extend_from_slice(&EXACT_A729_EXECUTABLE_SHA256);
    out.extend_from_slice(&PRODUCER_MANIFEST_SHA256);
    match &frame.observation {
        None => {
            out.push(0);
            out.extend_from_slice(&[0; 7]);
        }
        Some(PeaceConferenceObservation::Inactive { refresh_sequence }) => {
            out.push(1);
            out.extend_from_slice(&[0; 7]);
            u64v(&mut out, *refresh_sequence);
        }
        Some(PeaceConferenceObservation::NoConferenceInProgress) => {
            out.push(3);
            out.extend_from_slice(&[0; 7]);
        }
        Some(PeaceConferenceObservation::Active(active)) => {
            out.push(2);
            out.extend_from_slice(&[0; 7]);
            u64v(&mut out, active.refresh_sequence);
            u64v(&mut out, active.refresh_epoch);
            u32v(&mut out, active.conference_reference.object_type);
            u32v(&mut out, active.conference_reference.object_id);
            i32v(&mut out, active.current_turn_raw);
            out.push(u8::from(active.completed));
            out.push(u8::from(active.semantic_snapshot_complete));
            out.extend_from_slice(&[0; 2]);
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<PeaceConferenceTransportFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC
        || reader.u32()? != SCHEMA_VERSION
        || reader.u32()? != AUTHORITY_EXACT_A729_PEACE_CONFERENCE
    {
        return invalid("PCS1 magic/schema/authority mismatch");
    }
    let status_flags = reader.u32()?;
    let frame_hour = reader.u64()?;
    let elapsed_wall_ms = reader.u64()?;
    if reader.bytes(32)? != EXACT_A729_EXECUTABLE_SHA256
        || reader.bytes(32)? != PRODUCER_MANIFEST_SHA256
    {
        return invalid("PCS1 executable/producer authority mismatch");
    }
    let kind = reader.u8()?;
    if reader.bytes(7)? != [0; 7] {
        return invalid("PCS1 observation reserved bytes are nonzero");
    }
    let observation = match kind {
        0 => None,
        1 => Some(PeaceConferenceObservation::Inactive {
            refresh_sequence: reader.u64()?,
        }),
        2 => {
            let active = ActivePeaceConferenceObservation {
                refresh_sequence: reader.u64()?,
                refresh_epoch: reader.u64()?,
                conference_reference: PeaceConferenceReference {
                    object_type: reader.u32()?,
                    object_id: reader.u32()?,
                },
                current_turn_raw: reader.i32()?,
                completed: reader.bool()?,
                semantic_snapshot_complete: reader.bool()?,
            };
            if reader.bytes(2)? != [0; 2] {
                return invalid("PCS1 active reserved bytes are nonzero");
            }
            Some(PeaceConferenceObservation::Active(active))
        }
        3 => Some(PeaceConferenceObservation::NoConferenceInProgress),
        _ => return invalid("PCS1 observation kind is invalid"),
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = PeaceConferenceTransportFrame {
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        observation,
    };
    validate(&frame)?;
    Ok(frame)
}

fn validate(frame: &PeaceConferenceTransportFrame) -> Result<(), CarrierError> {
    match &frame.observation {
        None if frame.status_flags & MEASURED_EMPTY_STATUS != 0 => {
            invalid("PCS1 unavailable frame claims the source answered empty")
        }
        None if frame.status_flags != 0 => Ok(()),
        None => invalid("PCS1 clean frame omitted observation"),
        Some(PeaceConferenceObservation::NoConferenceInProgress)
            if frame.status_flags == MEASURED_EMPTY_STATUS =>
        {
            Ok(())
        }
        Some(PeaceConferenceObservation::NoConferenceInProgress) => {
            invalid("PCS1 measured-empty observation requires exactly the source-empty status")
        }
        Some(_) if frame.status_flags != 0 => {
            invalid("PCS1 unavailable frame contains an observation")
        }
        Some(PeaceConferenceObservation::Inactive { refresh_sequence })
            if *refresh_sequence == 0 =>
        {
            invalid("PCS1 inactive refresh sequence is zero")
        }
        Some(PeaceConferenceObservation::Active(active))
            if active.refresh_sequence == 0
                || active.refresh_epoch == 0
                || !active.conference_reference.valid()
                || active.current_turn_raw < 0 =>
        {
            invalid("PCS1 active observation has invalid exact identity/scalars")
        }
        Some(PeaceConferenceObservation::Active(active)) if active.semantic_snapshot_complete => {
            invalid("PCS1 v1 cannot claim a complete semantic snapshot")
        }
        Some(_) => Ok(()),
    }
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, len: usize) -> Result<&[u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, CarrierError> {
        Ok(self.bytes(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("PCS1 bool is not 0/1"),
        }
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, CarrierError> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(status_flags: u32) -> PeaceConferenceTransportFrame {
        PeaceConferenceTransportFrame {
            frame_hour: 12,
            elapsed_wall_ms: 34,
            status_flags,
            observation: Some(PeaceConferenceObservation::Active(
                ActivePeaceConferenceObservation {
                    refresh_sequence: 2,
                    refresh_epoch: 1,
                    conference_reference: PeaceConferenceReference {
                        object_type: 53,
                        object_id: 9,
                    },
                    current_turn_raw: 3,
                    completed: false,
                    semantic_snapshot_complete: false,
                },
            )),
        }
    }

    #[test]
    fn the_measured_zero_roundtrips_and_is_its_own_shape() {
        let payload = encode_no_conference_in_progress(12, 34).expect("the measured zero encodes");
        let decoded = decode_payload(&payload).expect("and decodes");
        assert_eq!(
            decoded.observation,
            Some(PeaceConferenceObservation::NoConferenceInProgress)
        );
        assert_eq!(decoded.status_flags, MEASURED_EMPTY_STATUS);
        assert_eq!(decoded.frame_hour, 12);
        assert_eq!(decoded.elapsed_wall_ms, 34);
    }

    #[test]
    fn the_measured_zero_is_not_a_failure() {
        assert_eq!(
            MEASURED_EMPTY_STATUS & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            0
        );
        assert!(!compact_contract::compact_status_is_failure(
            MEASURED_EMPTY_STATUS
        ));
    }

    #[test]
    fn a_measured_zero_may_not_ride_a_failure_word() {
        let mut frame = PeaceConferenceTransportFrame {
            frame_hour: 1,
            elapsed_wall_ms: 2,
            status_flags: MEASURED_EMPTY_STATUS,
            observation: Some(PeaceConferenceObservation::NoConferenceInProgress),
        };
        assert!(encode_payload(&frame).is_ok());
        frame.status_flags =
            MEASURED_EMPTY_STATUS | compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
        assert!(
            encode_payload(&frame).is_err(),
            "a measured zero may not ride a failure word"
        );
        frame.status_flags = 0;
        assert!(
            encode_payload(&frame).is_err(),
            "a measured zero is not a clean frame either; the bit IS the claim"
        );
    }

    #[test]
    fn a_refusal_may_never_wear_the_measured_empty_bit() {
        assert!(encode_source_unavailable(1, 2, MEASURED_EMPTY_STATUS).is_err());
        assert!(encode_payload(&PeaceConferenceTransportFrame {
            frame_hour: 1,
            elapsed_wall_ms: 2,
            status_flags: MEASURED_EMPTY_STATUS,
            observation: None,
        })
        .is_err());
    }

    #[test]
    fn the_older_kinds_are_untouched() {
        assert_eq!(SCHEMA_VERSION, 1, "kind 3 is additive, not a new language");
        let clean = active(0);
        assert_eq!(
            decode_payload(&encode_payload(&clean).unwrap()).unwrap(),
            clean
        );
        let inactive = PeaceConferenceTransportFrame {
            frame_hour: 5,
            elapsed_wall_ms: 6,
            status_flags: 0,
            observation: Some(PeaceConferenceObservation::Inactive {
                refresh_sequence: 4,
            }),
        };
        assert_eq!(
            decode_payload(&encode_payload(&inactive).unwrap()).unwrap(),
            inactive
        );
        assert!(
            encode_payload(&active(compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE)).is_err(),
            "an unavailable frame still may not carry an active observation"
        );
    }

    #[test]
    fn an_unknown_observation_kind_fails_closed() {
        let mut payload = encode_no_conference_in_progress(1, 2).unwrap();
        let kind_at = payload.len() - 8;
        payload[kind_at] = 4;
        assert!(decode_payload(&payload).is_err());
    }
}

fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i32v(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

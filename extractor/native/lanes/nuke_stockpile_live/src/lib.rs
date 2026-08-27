#[path = "../nuke_stockpile_exact.rs"]
mod exact;

pub use exact::{
    built_count_from_raw, read_country_built_nukes, read_country_nukes, CountryNukeSnapshot,
    NukeEntrySnapshot, NukeMemory, ReadError, COUNTRY_NUKE_ARRAY_OFF, COUNTRY_NUKE_CAPACITY_OFF,
    COUNTRY_NUKE_COUNT_OFF, MAX_NUKE_TYPE_ROWS, NUKE_AMOUNT_RAW_OFF, NUKE_OWNER_OFF,
    NUKE_READY_OFF, NUKE_STRIDE, NUKE_TYPE_COUNT, NUKE_TYPE_OFF, NUKE_VTABLE_RVA,
};

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 83;
pub const DOMAIN_NAME: &str = "nuke_stockpile_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"NUK1");
pub const SCHEMA_VERSION: u32 = 2;
pub const AUTHORITY_SOURCE_UNAVAILABLE: u32 = 0;
pub const AUTHORITY_EXACT_A729_RECEIPT: u32 = 1;
pub const MAX_COUNTRIES: usize = i32::MAX as usize;

const CARRIER_COUNTRY_BLOCK_LEN: usize = 12 + (NUKE_TYPE_COUNT as usize) * 24;

pub const PRODUCTION_AUTHORITY_ENABLED: bool = true;
pub const WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"0968ed85cd6103149ebd51415d6b64d1959bb05d1aff4cbfce9d31e437a6995c");
pub const WAVE5_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256: [u8; 32] =
    hex32(b"233962bab3a8ca79150977f1f7e77312d1bbd262e89e582d1c01caaf01a89032");
pub const WAVE5_VTABLE_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"cfd9d9e4088b4998398e579a5ebee222534868bad8a4529240b370faf467adbb");
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const RETAINED_PROGRESS_SAVE_SHA256: [u8; 32] =
    hex32(b"538f8a418e1662ed6759b06ed964687c4446cd50bc62b1fe63de210db0250073");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityState {
    SourceUnavailable,
    ExactA729Receipt,
}

compact_contract::wire_coded_codes! {
    no_discriminant AuthorityState: u32,
    code = raw,
    Ok = from_raw(value) -> Result<Self, CarrierError> { invalid("NUK1 authority state is unknown") };

    SourceUnavailable = AUTHORITY_SOURCE_UNAVAILABLE,
    ExactA729Receipt = AUTHORITY_EXACT_A729_RECEIPT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryRoot {
    pub tag_db_id: u32,
    pub object_ptr: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryNukeRow {
    pub tag_db_id: u32,
    pub container_capacity: i32,
    pub entries: Vec<NukeEntrySnapshot>,
}

impl CountryNukeRow {
    pub fn built_total(&self) -> i32 {
        self.entries.iter().fold(0i32, |total, entry| {
            total.wrapping_add(built_count_from_raw(entry.amount_raw))
        })
    }

    pub fn ready_total(&self) -> u64 {
        self.entries.iter().fold(0u64, |total, entry| {
            total.saturating_add(u64::from(entry.nukes_ready))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NukeStockpileSnapshot {
    pub source_sequence: u64,
    pub countries: Vec<CountryNukeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NukeStockpileFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub authority: AuthorityState,
    pub receipt_sha256: [u8; 32],
    pub snapshot: Option<NukeStockpileSnapshot>,
}

pub trait ExactNukeStockpileSource {
    fn is_exact_a729_country_nuke_source(&self) -> bool;
    fn read_nuke_stockpile(&self) -> Result<NukeStockpileSnapshot, CarrierError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CarrierError {
    AuthorityPending,
    Source(String),
    CountryRead {
        tag_db_id: u32,
        object_ptr: u64,
        source: Box<CarrierError>,
    },
    Memory(ReadError),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

impl From<ReadError> for CarrierError {
    fn from(value: ReadError) -> Self {
        Self::Memory(value)
    }
}

pub fn read_snapshot_from_country_roots<M: NukeMemory>(
    memory: &M,
    module_base: u64,
    source_sequence: u64,
    roots: &[CountryRoot],
) -> Result<NukeStockpileSnapshot, CarrierError> {
    if source_sequence == 0 || module_base == 0 || roots.len() > MAX_COUNTRIES {
        return invalid("country-root snapshot header is invalid");
    }
    let mut identities = BTreeSet::new();
    let mut countries = Vec::with_capacity(roots.len());
    for root in roots {
        if root.tag_db_id == 0
            || root.object_ptr == 0
            || !identities.insert((root.tag_db_id, root.object_ptr))
        {
            return invalid("country-root identity is zero or duplicated");
        }
        let snapshot =
            read_country_nukes(memory, root.object_ptr, module_base).map_err(|error| {
                CarrierError::CountryRead {
                    tag_db_id: root.tag_db_id,
                    object_ptr: root.object_ptr,
                    source: Box::new(CarrierError::Memory(error)),
                }
            })?;
        countries.push(CountryNukeRow {
            tag_db_id: root.tag_db_id,
            container_capacity: snapshot.container_capacity,
            entries: snapshot.entries,
        });
    }
    let snapshot = NukeStockpileSnapshot {
        source_sequence,
        countries,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn collect_and_encode<S: ExactNukeStockpileSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_AUTHORITY_ENABLED {
        return Err(CarrierError::AuthorityPending);
    }
    if !source.is_exact_a729_country_nuke_source() {
        return Err(CarrierError::Source(
            "source did not authenticate the exact a729 country-owned CNuke array".into(),
        ));
    }
    let snapshot = source.read_nuke_stockpile()?;
    encode_payload(&NukeStockpileFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        authority: AuthorityState::ExactA729Receipt,
        receipt_sha256: WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256,
        snapshot: Some(snapshot),
    })
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&NukeStockpileFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        authority: AuthorityState::SourceUnavailable,
        receipt_sha256: [0; 32],
        snapshot: None,
    })
}

pub fn validate_snapshot(snapshot: &NukeStockpileSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0 || snapshot.countries.len() > MAX_COUNTRIES {
        return invalid("nuke snapshot sequence/count is invalid");
    }
    let mut countries = BTreeSet::new();
    for country in &snapshot.countries {
        if country.tag_db_id == 0 || !countries.insert(country.tag_db_id) {
            return invalid("nuke snapshot has zero or duplicate country identity");
        }
        if country.container_capacity < NUKE_TYPE_COUNT {
            return invalid("country nuke container capacity violates exact a729 invariant");
        }
        if country.entries.len() != NUKE_TYPE_COUNT as usize {
            return invalid("country nuke entry count violates exact a729 invariant");
        }
        if country
            .entries
            .iter()
            .enumerate()
            .any(|(index, entry)| entry.nuke_type_raw != index as u32)
        {
            return invalid("country nuke type order violates exact a729 invariant");
        }
    }
    Ok(())
}

pub fn validate_frame(frame: &NukeStockpileFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::SourceUnavailable => {
            if frame.status_flags == 0
                || frame.receipt_sha256 != [0; 32]
                || frame.snapshot.is_some()
            {
                return invalid("unavailable A17 frame must be receipt-free and rowless");
            }
        }
        AuthorityState::ExactA729Receipt => {
            if frame.status_flags != 0
                || frame.receipt_sha256 != WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256
                || frame.snapshot.is_none()
            {
                return invalid("receipted A17 frame must be clean and measured");
            }
            validate_snapshot(frame.snapshot.as_ref().unwrap())?;
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &NukeStockpileFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, frame.authority.raw());
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&frame.receipt_sha256);
    out.extend_from_slice(&WAVE5_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256);
    out.extend_from_slice(&WAVE5_VTABLE_ARTIFACT_SHA256);
    out.push(u8::from(frame.snapshot.is_some()));
    out.extend_from_slice(&[0; 3]);
    let Some(snapshot) = &frame.snapshot else {
        return Ok(out);
    };
    u64v(&mut out, snapshot.source_sequence);
    u32v(&mut out, snapshot.countries.len() as u32);
    for country in &snapshot.countries {
        u32v(&mut out, country.tag_db_id);
        i32v(&mut out, country.container_capacity);
        u32v(&mut out, country.entries.len() as u32);
        for entry in &country.entries {
            u32v(&mut out, entry.nuke_type_raw);
            u32v(&mut out, 0);
            i64v(&mut out, entry.amount_raw);
            u32v(&mut out, entry.nukes_ready);
            u32v(&mut out, 0);
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<NukeStockpileFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("NUK1 magic/schema mismatch");
    }
    let authority = AuthorityState::from_raw(reader.u32()?)?;
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    if reader.bytes(32)? != EXECUTABLE_SHA256 {
        return invalid("NUK1 executable identity mismatch");
    }
    let mut receipt_sha256 = [0; 32];
    receipt_sha256.copy_from_slice(reader.bytes(32)?);
    if reader.bytes(32)? != WAVE5_CONTAINER_CONSTRUCTOR_MANIFEST_SHA256 {
        return invalid("NUK1 container-constructor authority mismatch");
    }
    if reader.bytes(32)? != WAVE5_VTABLE_ARTIFACT_SHA256 {
        return invalid("NUK1 CNuke vtable authority mismatch");
    }
    let has_snapshot = reader.bool()?;
    if reader.bytes(3)? != [0, 0, 0] {
        return invalid("NUK1 availability reserved bytes are nonzero");
    }
    let snapshot = if has_snapshot {
        let source_sequence = reader.u64()?;
        let country_count = reader.u32()? as usize;
        if country_count > MAX_COUNTRIES {
            return invalid("NUK1 country count exceeds bound");
        }
        if country_count > reader.remaining() / CARRIER_COUNTRY_BLOCK_LEN {
            return invalid("NUK1 country count exceeds the payload's capacity");
        }
        let mut countries = Vec::with_capacity(country_count);
        for _ in 0..country_count {
            let tag_db_id = reader.u32()?;
            let container_capacity = reader.i32()?;
            let entry_count = reader.u32()? as usize;
            if entry_count != NUKE_TYPE_COUNT as usize {
                return invalid("NUK1 entry count violates exact a729 invariant");
            }
            let mut entries = Vec::with_capacity(entry_count);
            for _ in 0..entry_count {
                let nuke_type_raw = reader.u32()?;
                if reader.u32()? != 0 {
                    return invalid("NUK1 entry identity reserved field is nonzero");
                }
                let amount_raw = reader.i64()?;
                let nukes_ready = reader.u32()?;
                if reader.u32()? != 0 {
                    return invalid("NUK1 entry reserved field is nonzero");
                }
                entries.push(NukeEntrySnapshot {
                    nuke_type_raw,
                    amount_raw,
                    nukes_ready,
                });
            }
            countries.push(CountryNukeRow {
                tag_db_id,
                container_capacity,
                entries,
            });
        }
        Some(NukeStockpileSnapshot {
            source_sequence,
            countries,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = NukeStockpileFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        authority,
        receipt_sha256,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i64v(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn i32v(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn remaining(&self) -> usize {
        self.payload.len().saturating_sub(self.at)
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.bytes(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("NUK1 bool byte is invalid"),
        }
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, CarrierError> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, CarrierError> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct MissingMemory;

    impl NukeMemory for MissingMemory {
        fn read_u32(&self, address: u64) -> Result<u32, ReadError> {
            Err(ReadError::Memory(address))
        }

        fn read_u64(&self, address: u64) -> Result<u64, ReadError> {
            Err(ReadError::Memory(address))
        }
    }

    #[test]
    fn country_walk_failure_retains_country_and_exact_address() {
        let error = read_snapshot_from_country_roots(
            &MissingMemory,
            0x1400_0000_0,
            1,
            &[CountryRoot {
                tag_db_id: 23,
                object_ptr: 0x2000_0000,
            }],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CarrierError::CountryRead {
                tag_db_id: 23,
                object_ptr: 0x2000_0000,
                source,
            } if matches!(
                *source,
                CarrierError::Memory(ReadError::Memory(0x2000_1310))
            )
        ));
    }

    fn receipted_frame() -> NukeStockpileFrame {
        NukeStockpileFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            authority: AuthorityState::ExactA729Receipt,
            receipt_sha256: WAVE5_PRODUCER_CONSUMER_MANIFEST_SHA256,
            snapshot: Some(NukeStockpileSnapshot {
                source_sequence: 17,
                countries: vec![CountryNukeRow {
                    tag_db_id: 23,
                    container_capacity: 2,
                    entries: vec![
                        NukeEntrySnapshot {
                            nuke_type_raw: 0,
                            amount_raw: 8_000,
                            nukes_ready: 0,
                        },
                        NukeEntrySnapshot {
                            nuke_type_raw: 1,
                            amount_raw: 250_000_000,
                            nukes_ready: 1,
                        },
                    ],
                }],
            }),
        }
    }

    #[test]
    fn raw_amount_and_ready_roundtrip_without_fudge_conversion() {
        let expected = receipted_frame();
        let bytes = encode_payload(&expected).unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(decoded, expected);
        let row = &decoded.snapshot.unwrap().countries[0];
        assert_eq!(row.entries[0].amount_raw, 8_000);
        assert_eq!(row.built_total(), 2);
        assert_eq!(row.ready_total(), 1);
    }

    #[test]
    fn unavailable_authority_cannot_carry_measured_rows() {
        let mut frame = receipted_frame();
        frame.authority = AuthorityState::SourceUnavailable;
        frame.receipt_sha256 = [0; 32];
        assert!(encode_payload(&frame).is_err());
        assert!(encode_source_unavailable(900, 7, 0).is_err());
        assert!(
            decode_payload(&encode_source_unavailable(900, 7, 1).unwrap())
                .unwrap()
                .snapshot
                .is_none()
        );
    }

    #[test]
    fn schema_v2_rejects_noncanonical_capacity_count_and_type_order() {
        let mut frame = receipted_frame();
        frame.snapshot.as_mut().unwrap().countries[0].container_capacity = 1;
        assert!(encode_payload(&frame).is_err());

        let mut frame = receipted_frame();
        frame.snapshot.as_mut().unwrap().countries[0].entries.pop();
        assert!(encode_payload(&frame).is_err());

        let mut frame = receipted_frame();
        frame.snapshot.as_mut().unwrap().countries[0].entries[0].nuke_type_raw = 1;
        assert!(encode_payload(&frame).is_err());
    }

    struct NeverRead(Cell<bool>);
    impl ExactNukeStockpileSource for NeverRead {
        fn is_exact_a729_country_nuke_source(&self) -> bool {
            self.0.set(true);
            true
        }
        fn read_nuke_stockpile(&self) -> Result<NukeStockpileSnapshot, CarrierError> {
            self.0.set(true);
            Err(CarrierError::Source("must not be reached".into()))
        }
    }

    #[test]
    fn production_source_gate_consults_authenticated_source() {
        let source = NeverRead(Cell::new(false));
        assert_eq!(
            collect_and_encode(&source, 900, 7),
            Err(CarrierError::Source("must not be reached".into()))
        );
        assert!(source.0.get());
    }

    struct Unauthenticated;
    impl ExactNukeStockpileSource for Unauthenticated {
        fn is_exact_a729_country_nuke_source(&self) -> bool {
            false
        }
        fn read_nuke_stockpile(&self) -> Result<NukeStockpileSnapshot, CarrierError> {
            panic!("unauthenticated source must not be read")
        }
    }

    #[test]
    fn unauthenticated_source_fails_before_memory_read() {
        assert!(matches!(
            collect_and_encode(&Unauthenticated, 900, 7),
            Err(CarrierError::Source(message))
                if message.contains("did not authenticate")
        ));
    }
}

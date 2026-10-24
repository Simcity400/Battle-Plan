//! Allocation-free terminal evidence, prepared before collector hooks are enabled.

use std::io;

use crate::codec::invalid_static;

pub const WITNESS_FILE_NAME: &str = "collector-failure.bin";
pub const WITNESS_BYTES: usize = 256;
const MAGIC: &[u8; 8] = b"H4OOM\0\0\0";
const VERSION: u16 = 2;
const CHECKSUM_OFFSET: usize = WITNESS_BYTES - 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FailurePhase {
    Bootstrap = 1,
    Callback,
    Acquisition,
    Raw,
    Decode,
    Extended,
    Spool,
    Encode,
    Map,
    Status,
    Teardown,
}

impl FailurePhase {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "bootstrap" => Self::Bootstrap,
            "callback" => Self::Callback,
            "acquisition" => Self::Acquisition,
            "raw" => Self::Raw,
            "decode" => Self::Decode,
            "extended" => Self::Extended,
            "spool" => Self::Spool,
            "encode" => Self::Encode,
            "map" => Self::Map,
            "status" => Self::Status,
            "teardown" => Self::Teardown,
            _ => return None,
        })
    }

    fn valid_byte(byte: u8) -> bool {
        (Self::Bootstrap as u8..=Self::Teardown as u8).contains(&byte)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FailureCode {
    Reserve = 1,
    Box,
    SizeOverflow,
    Thread,
    Native,
}

impl FailureCode {
    fn valid_byte(byte: u8) -> bool {
        (Self::Reserve as u8..=Self::Native as u8).contains(&byte)
    }
}

/// Hours an ordered writer admitted in one context that no spool commit covers yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UncommittedRange {
    pub campaign_id: [u8; 36],
    pub branch: u64,
    pub load_signal: Option<i64>,
    pub first_hour: i32,
    pub last_hour: i32,
}

impl UncommittedRange {
    pub fn is_valid(&self) -> bool {
        self.first_hour <= self.last_hour && crate::identity::is_campaign_uuid(&self.campaign_id)
    }
}

/// A complete spool watermark is evidence of publication, never an inferred hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalWitness {
    pub session: u64,
    pub process_start_time: u64,
    pub campaign_id: Option<[u8; 36]>,
    pub branch: Option<u64>,
    pub load_signal: Option<i64>,
    pub failed_hour: Option<i32>,
    pub committed_campaign_id: Option<[u8; 36]>,
    pub committed_segment: Option<u32>,
    pub committed_bytes: u64,
    pub pending_campaign_id: Option<[u8; 36]>,
    pub pending_segment: Option<u32>,
    pub pending_bytes: u64,
    /// Independent of the attempted context: it may name another campaign or branch.
    pub uncommitted: Option<UncommittedRange>,
    pub phase: u8,
    pub code: u8,
    pub drain_complete: bool,
}

impl TerminalWitness {
    /// The recorder supplies identity before the collector's first heap operation.
    pub fn prepared(session: u64, process_start_time: u64) -> [u8; WITNESS_BYTES] {
        Self {
            session,
            process_start_time,
            campaign_id: None,
            branch: None,
            load_signal: None,
            failed_hour: None,
            committed_campaign_id: None,
            committed_segment: None,
            committed_bytes: 0,
            pending_campaign_id: None,
            pending_segment: None,
            pending_bytes: 0,
            uncommitted: None,
            phase: 0,
            code: 0,
            drain_complete: false,
        }
        .encode()
    }

    pub fn decode_prepared(bytes: &[u8]) -> io::Result<(u64, u64)> {
        let prepared = Self::decode_inner(bytes, true)?;
        Ok((prepared.session, prepared.process_start_time))
    }

    pub fn encode(&self) -> [u8; WITNESS_BYTES] {
        let mut bytes = [0; WITNESS_BYTES];
        self.encode_into(&mut bytes);
        bytes
    }

    pub fn encode_into(&self, bytes: &mut [u8; WITNESS_BYTES]) {
        bytes.fill(0);
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
        bytes[10] = u8::from(self.campaign_id.is_some())
            | (u8::from(self.branch.is_some()) << 1)
            | (u8::from(self.load_signal.is_some()) << 2)
            | (u8::from(self.failed_hour.is_some()) << 3)
            | (u8::from(self.committed_segment.is_some()) << 4)
            | (u8::from(self.drain_complete) << 5)
            | (u8::from(self.committed_campaign_id.is_some()) << 6)
            | (u8::from(self.pending_campaign_id.is_some()) << 7);
        bytes[11] = self.phase;
        bytes[12] = self.code;
        bytes[13] = u8::from(self.pending_segment.is_some())
            | (u8::from(self.uncommitted.is_some()) << 1)
            | (u8::from(
                self.uncommitted
                    .is_some_and(|range| range.load_signal.is_some()),
            ) << 2);
        bytes[16..24].copy_from_slice(&self.session.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.process_start_time.to_le_bytes());
        if let Some(campaign) = self.campaign_id {
            bytes[32..68].copy_from_slice(&campaign);
        }
        if let Some(branch) = self.branch {
            bytes[68..76].copy_from_slice(&branch.to_le_bytes());
        }
        if let Some(signal) = self.load_signal {
            bytes[76..84].copy_from_slice(&signal.to_le_bytes());
        }
        if let Some(hour) = self.failed_hour {
            bytes[84..88].copy_from_slice(&hour.to_le_bytes());
        }
        if let Some(segment) = self.committed_segment {
            bytes[88..92].copy_from_slice(&segment.to_le_bytes());
        }
        bytes[92..100].copy_from_slice(&self.committed_bytes.to_le_bytes());
        if let Some(campaign) = self.committed_campaign_id {
            bytes[100..136].copy_from_slice(&campaign);
        }
        if let Some(campaign) = self.pending_campaign_id {
            bytes[136..172].copy_from_slice(&campaign);
        }
        if let Some(segment) = self.pending_segment {
            bytes[172..176].copy_from_slice(&segment.to_le_bytes());
        }
        bytes[176..184].copy_from_slice(&self.pending_bytes.to_le_bytes());
        if let Some(range) = self.uncommitted {
            bytes[184..220].copy_from_slice(&range.campaign_id);
            bytes[220..228].copy_from_slice(&range.branch.to_le_bytes());
            if let Some(signal) = range.load_signal {
                bytes[228..236].copy_from_slice(&signal.to_le_bytes());
            }
            bytes[236..240].copy_from_slice(&range.first_hour.to_le_bytes());
            bytes[240..244].copy_from_slice(&range.last_hour.to_le_bytes());
        }
        let checksum = crc32fast::hash(&bytes[..CHECKSUM_OFFSET]);
        bytes[CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());
    }

    pub fn encode_bundle_record(
        &self,
        bytes: &mut [u8; WITNESS_BYTES],
        record_index: u32,
        total_count: u32,
    ) {
        self.encode_into(bytes);
        if total_count > 1 {
            bytes[244..248].copy_from_slice(&total_count.to_le_bytes());
            bytes[248..252].copy_from_slice(&record_index.to_le_bytes());
            let checksum = crc32fast::hash(&bytes[..CHECKSUM_OFFSET]);
            bytes[CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());
        }
    }

    pub fn bundle_count(bytes: &[u8]) -> io::Result<usize> {
        Self::decode(bytes)?;
        let count = u32::from_le_bytes(bytes[244..248].try_into().unwrap());
        Ok(count.max(1) as usize)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        Self::decode_inner(bytes, false)
    }

    fn decode_inner(bytes: &[u8], prepared: bool) -> io::Result<Self> {
        if bytes.len() != WITNESS_BYTES
            || &bytes[..8] != MAGIC
            || u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != VERSION
            || bytes[13] & !7 != 0
            || (if prepared {
                bytes[10] != 0 || bytes[11] != 0 || bytes[12] != 0 || bytes[13] != 0
            } else {
                bytes[11] == 0 || bytes[12] == 0
            })
            || bytes[14..16].iter().any(|byte| *byte != 0)
            || (if prepared {
                bytes[244..CHECKSUM_OFFSET].iter().any(|byte| *byte != 0)
            } else {
                let count = u32::from_le_bytes(bytes[244..248].try_into().unwrap());
                let index = u32::from_le_bytes(bytes[248..252].try_into().unwrap());
                (count == 0 && index != 0) || count == 1 || (count > 1 && index >= count)
            })
            || crc32fast::hash(&bytes[..CHECKSUM_OFFSET])
                != u32::from_le_bytes(bytes[CHECKSUM_OFFSET..].try_into().unwrap())
        {
            return Err(invalid_static("invalid collector terminal witness"));
        }
        if !prepared
            && (!FailurePhase::valid_byte(bytes[11]) || !FailureCode::valid_byte(bytes[12]))
        {
            return Err(invalid_static(
                "unsupported collector terminal failure code",
            ));
        }
        let flags = bytes[10];
        for (mask, range) in [
            (1, 32..68),
            (2, 68..76),
            (4, 76..84),
            (8, 84..88),
            (16, 88..92),
            (64, 100..136),
            (128, 136..172),
        ] {
            if flags & mask == 0 && bytes[range].iter().any(|byte| *byte != 0) {
                return Err(invalid_static("noncanonical collector terminal witness"));
            }
        }
        if bytes[13] & 1 == 0 && bytes[172..176].iter().any(|byte| *byte != 0)
            || bytes[13] & 2 == 0
                && (bytes[13] & 4 != 0 || bytes[184..244].iter().any(|byte| *byte != 0))
            || bytes[13] & 4 == 0 && bytes[228..236].iter().any(|byte| *byte != 0)
        {
            return Err(invalid_static("noncanonical collector terminal witness"));
        }
        let witness = Self {
            session: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            process_start_time: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            campaign_id: (flags & 1 != 0).then(|| bytes[32..68].try_into().unwrap()),
            branch: (flags & 2 != 0).then(|| u64::from_le_bytes(bytes[68..76].try_into().unwrap())),
            load_signal: (flags & 4 != 0)
                .then(|| i64::from_le_bytes(bytes[76..84].try_into().unwrap())),
            failed_hour: (flags & 8 != 0)
                .then(|| i32::from_le_bytes(bytes[84..88].try_into().unwrap())),
            committed_campaign_id: (flags & 64 != 0).then(|| bytes[100..136].try_into().unwrap()),
            committed_segment: (flags & 16 != 0)
                .then(|| u32::from_le_bytes(bytes[88..92].try_into().unwrap())),
            committed_bytes: u64::from_le_bytes(bytes[92..100].try_into().unwrap()),
            pending_campaign_id: (flags & 128 != 0).then(|| bytes[136..172].try_into().unwrap()),
            pending_segment: (bytes[13] & 1 != 0)
                .then(|| u32::from_le_bytes(bytes[172..176].try_into().unwrap())),
            pending_bytes: u64::from_le_bytes(bytes[176..184].try_into().unwrap()),
            uncommitted: (bytes[13] & 2 != 0).then(|| UncommittedRange {
                campaign_id: bytes[184..220].try_into().unwrap(),
                branch: u64::from_le_bytes(bytes[220..228].try_into().unwrap()),
                load_signal: (bytes[13] & 4 != 0)
                    .then(|| i64::from_le_bytes(bytes[228..236].try_into().unwrap())),
                first_hour: i32::from_le_bytes(bytes[236..240].try_into().unwrap()),
                last_hour: i32::from_le_bytes(bytes[240..244].try_into().unwrap()),
            }),
            phase: bytes[11],
            code: bytes[12],
            drain_complete: flags & 32 != 0,
        };
        if witness.committed_segment.is_none() && witness.committed_bytes != 0
            || witness.committed_segment.is_some() != witness.committed_campaign_id.is_some()
            || witness.pending_segment.is_none() && witness.pending_bytes != 0
            || witness.pending_segment.is_some() && witness.pending_campaign_id.is_none()
            || witness
                .campaign_id
                .is_some_and(|id| !crate::identity::is_campaign_uuid(&id))
            || witness
                .committed_campaign_id
                .is_some_and(|id| !crate::identity::is_campaign_uuid(&id))
            || witness
                .pending_campaign_id
                .is_some_and(|id| !crate::identity::is_campaign_uuid(&id))
            || witness.uncommitted.is_some_and(|range| !range.is_valid())
        {
            return Err(invalid_static("invalid collector terminal context"));
        }
        Ok(witness)
    }
}

pub fn decode_bundle(bytes: &[u8]) -> io::Result<Vec<TerminalWitness>> {
    let first = bytes
        .get(..WITNESS_BYTES)
        .ok_or_else(|| invalid_static("incomplete collector terminal bundle"))?;
    let count = TerminalWitness::bundle_count(first)?;
    if count.checked_mul(WITNESS_BYTES) != Some(bytes.len()) {
        return Err(invalid_static("incomplete collector terminal bundle"));
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(count)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    for (index, bytes) in bytes.chunks_exact(WITNESS_BYTES).enumerate() {
        let record = TerminalWitness::decode(bytes)?;
        let encoded_index = u32::from_le_bytes(bytes[248..252].try_into().unwrap()) as usize;
        if TerminalWitness::bundle_count(bytes)? != count
            || encoded_index != index
            || records.first().is_some_and(|first: &TerminalWitness| {
                record.session != first.session
                    || record.process_start_time != first.process_start_time
                    || record.phase != first.phase
                    || record.code != first.code
                    || record.drain_complete != first.drain_complete
                    || record.failed_hour.is_some()
            })
        {
            return Err(invalid_static("inconsistent collector terminal bundle"));
        }
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn witness() -> TerminalWitness {
        TerminalWitness {
            session: 17,
            process_start_time: 123,
            campaign_id: Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6"),
            branch: Some(9),
            load_signal: Some(-7),
            failed_hour: Some(-3),
            committed_campaign_id: Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6"),
            committed_segment: Some(2),
            committed_bytes: 4567,
            pending_campaign_id: None,
            pending_segment: None,
            pending_bytes: 0,
            uncommitted: None,
            phase: 2,
            code: 1,
            drain_complete: true,
        }
    }

    fn range() -> UncommittedRange {
        UncommittedRange {
            campaign_id: *b"e64c5dda-6a12-487f-9d05-2a533f2925a4",
            branch: 11,
            load_signal: Some(-5),
            first_hour: -2,
            last_hour: 40,
        }
    }

    fn resealed(mut bytes: [u8; WITNESS_BYTES]) -> [u8; WITNESS_BYTES] {
        let crc = crc32fast::hash(&bytes[..CHECKSUM_OFFSET]);
        bytes[CHECKSUM_OFFSET..].copy_from_slice(&crc.to_le_bytes());
        bytes
    }

    #[test]
    fn terminal_witness_keeps_uncommitted_range_independent_of_attempt_and_watermarks() {
        let attempts = [
            witness(),
            TerminalWitness {
                campaign_id: None,
                branch: None,
                load_signal: None,
                failed_hour: None,
                committed_campaign_id: None,
                committed_segment: None,
                committed_bytes: 0,
                ..witness()
            },
        ];
        for uncommitted in [
            range(),
            UncommittedRange {
                load_signal: None,
                ..range()
            },
            UncommittedRange {
                first_hour: 40,
                ..range()
            },
            UncommittedRange {
                campaign_id: *b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6",
                branch: 9,
                first_hour: i32::MIN,
                last_hour: i32::MAX,
                ..range()
            },
        ] {
            for attempted in attempts {
                let witness = TerminalWitness {
                    uncommitted: Some(uncommitted),
                    ..attempted
                };
                assert_eq!(TerminalWitness::decode(&witness.encode()).unwrap(), witness);
                let mut buffer = [u8::MAX; WITNESS_BYTES];
                witness.encode_into(&mut buffer);
                assert_eq!(buffer, witness.encode());
            }
        }
    }

    #[test]
    fn terminal_witness_refuses_invalid_or_noncanonical_uncommitted_ranges() {
        for uncommitted in [
            UncommittedRange {
                first_hour: 41,
                ..range()
            },
            UncommittedRange {
                campaign_id: [b'x'; 36],
                ..range()
            },
            UncommittedRange {
                campaign_id: [0; 36],
                ..range()
            },
        ] {
            let witness = TerminalWitness {
                uncommitted: Some(uncommitted),
                ..witness()
            };
            assert!(TerminalWitness::decode(&witness.encode()).is_err());
        }
        let ranged = TerminalWitness {
            uncommitted: Some(range()),
            ..witness()
        }
        .encode();
        for mask in [2, 4] {
            let mut noncanonical = ranged;
            noncanonical[13] &= !mask;
            assert!(TerminalWitness::decode(&resealed(noncanonical)).is_err());
        }
        let bytes = witness().encode();
        for (byte, value) in [
            (13, 4),
            (13, 8),
            (184, 1),
            (220, 1),
            (229, 1),
            (243, 1),
            (250, 1),
        ] {
            let mut noncanonical = bytes;
            noncanonical[byte] |= value;
            assert!(
                TerminalWitness::decode(&resealed(noncanonical)).is_err(),
                "byte {byte}"
            );
        }
        let mut unsigned = ranged;
        unsigned[13] &= !4;
        unsigned[228..236].fill(0);
        assert_eq!(
            TerminalWitness::decode(&resealed(unsigned))
                .unwrap()
                .uncommitted,
            Some(UncommittedRange {
                load_signal: None,
                ..range()
            })
        );
        let mut prepared = TerminalWitness::prepared(17, 123);
        prepared[13] = 2;
        assert!(TerminalWitness::decode_prepared(&resealed(prepared)).is_err());
    }

    #[test]
    fn terminal_bundle_requires_every_counted_record_in_order_with_matching_identity() {
        let primary = witness();
        let extension = TerminalWitness {
            campaign_id: None,
            branch: None,
            load_signal: None,
            failed_hour: None,
            uncommitted: Some(range()),
            ..primary
        };
        let mut bytes = [0; 3 * WITNESS_BYTES];
        for (index, record) in [primary, extension, extension].into_iter().enumerate() {
            let output = (&mut bytes[index * WITNESS_BYTES..(index + 1) * WITNESS_BYTES])
                .try_into()
                .unwrap();
            record.encode_bundle_record(output, index as u32, 3);
        }
        assert_eq!(
            decode_bundle(&bytes).unwrap(),
            [primary, extension, extension]
        );
        assert_eq!(
            TerminalWitness::bundle_count(&bytes[..WITNESS_BYTES]).unwrap(),
            3
        );
        assert_eq!(decode_bundle(&primary.encode()).unwrap(), [primary]);
        for end in 0..bytes.len() {
            assert!(decode_bundle(&bytes[..end]).is_err());
        }
        for invalid in [
            TerminalWitness {
                session: 18,
                ..extension
            },
            TerminalWitness {
                process_start_time: 124,
                ..extension
            },
            TerminalWitness {
                phase: 3,
                ..extension
            },
            TerminalWitness {
                code: 2,
                ..extension
            },
            TerminalWitness {
                failed_hour: Some(5),
                ..extension
            },
        ] {
            let mut changed = bytes;
            invalid.encode_bundle_record(
                (&mut changed[WITNESS_BYTES..2 * WITNESS_BYTES])
                    .try_into()
                    .unwrap(),
                1,
                3,
            );
            assert!(decode_bundle(&changed).is_err());
        }
        let mut duplicate = bytes;
        duplicate[2 * WITNESS_BYTES..].copy_from_slice(&bytes[WITNESS_BYTES..2 * WITNESS_BYTES]);
        assert!(decode_bundle(&duplicate).is_err());
        let mut count_mismatch = bytes;
        extension.encode_bundle_record(
            (&mut count_mismatch[WITNESS_BYTES..2 * WITNESS_BYTES])
                .try_into()
                .unwrap(),
            1,
            2,
        );
        assert!(decode_bundle(&count_mismatch).is_err());
    }

    #[test]
    fn terminal_witness_preserves_context_without_heap_storage() {
        for witness in [
            witness(),
            TerminalWitness {
                campaign_id: None,
                branch: None,
                load_signal: None,
                failed_hour: None,
                committed_campaign_id: None,
                committed_segment: None,
                committed_bytes: 0,
                drain_complete: false,
                ..witness()
            },
        ] {
            assert_eq!(TerminalWitness::decode(&witness.encode()).unwrap(), witness);
            let mut buffer = [u8::MAX; WITNESS_BYTES];
            witness.encode_into(&mut buffer);
            assert_eq!(buffer, witness.encode());
        }
    }

    #[test]
    fn terminal_witness_keeps_previous_campaign_watermark_independent_of_attempt() {
        let previous_campaign = *b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6";
        for attempted_campaign in [Some(*b"ae7ba7cb-5218-42f4-a7ba-51a3dd74c964"), None] {
            let witness = TerminalWitness {
                campaign_id: attempted_campaign,
                committed_campaign_id: Some(previous_campaign),
                ..witness()
            };
            assert_eq!(TerminalWitness::decode(&witness.encode()).unwrap(), witness);
        }
    }

    #[test]
    fn terminal_witness_caps_pending_campaign_independently_of_last_commit() {
        for pending_segment in [Some(0), None] {
            let witness = TerminalWitness {
                campaign_id: Some(*b"ae7ba7cb-5218-42f4-a7ba-51a3dd74c964"),
                pending_campaign_id: Some(*b"e64c5dda-6a12-487f-9d05-2a533f2925a4"),
                pending_segment,
                pending_bytes: if pending_segment.is_some() { 1234 } else { 0 },
                ..witness()
            };
            assert_eq!(TerminalWitness::decode(&witness.encode()).unwrap(), witness);
        }
    }

    #[test]
    fn prepared_witness_is_identity_only_and_never_terminal_failure() {
        let bytes = TerminalWitness::prepared(17, 123);
        assert_eq!(TerminalWitness::decode_prepared(&bytes).unwrap(), (17, 123));
        assert!(TerminalWitness::decode(&bytes).is_err());
        assert!(TerminalWitness::decode_prepared(&witness().encode()).is_err());
        for index in 0..WITNESS_BYTES {
            let mut damaged = bytes;
            damaged[index] ^= 1;
            assert!(TerminalWitness::decode_prepared(&damaged).is_err());
        }
    }

    #[test]
    fn terminal_witness_rejects_every_corrupted_or_incomplete_byte() {
        let bytes = witness().encode();
        for end in 0..WITNESS_BYTES {
            assert!(TerminalWitness::decode(&bytes[..end]).is_err());
        }
        for index in 0..WITNESS_BYTES {
            let mut damaged = bytes;
            damaged[index] ^= 1;
            assert!(TerminalWitness::decode(&damaged).is_err(), "byte {index}");
        }
        for flag in [1, 2, 4, 8, 16, 64] {
            let mut noncanonical = bytes;
            noncanonical[10] &= !flag;
            let crc = crc32fast::hash(&noncanonical[..CHECKSUM_OFFSET]);
            noncanonical[CHECKSUM_OFFSET..].copy_from_slice(&crc.to_le_bytes());
            assert!(TerminalWitness::decode(&noncanonical).is_err());
        }
        let pending = TerminalWitness {
            pending_campaign_id: witness().campaign_id,
            pending_segment: Some(3),
            pending_bytes: 10,
            ..witness()
        }
        .encode();
        for byte in [10, 13] {
            let mut noncanonical = pending;
            noncanonical[byte] &= if byte == 10 { !128 } else { !1 };
            let crc = crc32fast::hash(&noncanonical[..CHECKSUM_OFFSET]);
            noncanonical[CHECKSUM_OFFSET..].copy_from_slice(&crc.to_le_bytes());
            assert!(TerminalWitness::decode(&noncanonical).is_err());
        }
    }

    #[test]
    fn terminal_witness_requires_campaign_for_every_committed_watermark() {
        for witness in [
            TerminalWitness {
                committed_campaign_id: None,
                ..witness()
            },
            TerminalWitness {
                committed_segment: None,
                committed_bytes: 0,
                ..witness()
            },
            TerminalWitness {
                committed_campaign_id: Some([b'x'; 36]),
                ..witness()
            },
            TerminalWitness {
                pending_segment: Some(3),
                ..witness()
            },
            TerminalWitness {
                pending_bytes: 10,
                ..witness()
            },
            TerminalWitness {
                pending_campaign_id: Some([b'x'; 36]),
                ..witness()
            },
        ] {
            assert!(TerminalWitness::decode(&witness.encode()).is_err());
        }
    }

    #[test]
    fn terminal_witness_rejects_unknown_failure_codes_even_with_valid_crc() {
        for (byte, unknown) in [
            (11, FailurePhase::Teardown as u8 + 1),
            (12, FailureCode::Native as u8 + 1),
            (11, u8::MAX),
            (12, u8::MAX),
        ] {
            let mut unsupported = witness().encode();
            unsupported[byte] = unknown;
            let crc = crc32fast::hash(&unsupported[..CHECKSUM_OFFSET]);
            unsupported[CHECKSUM_OFFSET..].copy_from_slice(&crc.to_le_bytes());
            assert!(TerminalWitness::decode(&unsupported).is_err());
        }
        for phase in FailurePhase::Bootstrap as u8..=FailurePhase::Teardown as u8 {
            for code in FailureCode::Reserve as u8..=FailureCode::Native as u8 {
                let witness = TerminalWitness {
                    phase,
                    code,
                    ..witness()
                };
                assert_eq!(TerminalWitness::decode(&witness.encode()).unwrap(), witness);
            }
        }
    }
}

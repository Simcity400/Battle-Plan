//! The session state-topology census (H4SC): the complete list of state ids
//! the running game's map holds, written ONCE per logical stream.
//!
//! Owner ruling 2026-08-20: the recording writes the complete state-id list
//! once at the start of the recording, like the content-identity census —
//! never per hour. The replay uses it to show a definite "0 buildings" for a
//! state whose scan came back empty, instead of a "no data" label: a d48
//! frame whose `scanned_states` equals this census has measured the WHOLE
//! map, so absence in it is absence in the world.
//!
//! The map's state roster is fixed at load, so one record per logical stream
//! is the honest cardinality; a rollover re-publishes the same payload so
//! every independently consumable stream carries its own copy, exactly as
//! the content-identity census does.

use crate::record_semantics::KnownRecordType;
use id_set_codec::{decode_set_block, encode_set_block, MAX_DECODABLE_IDS};

pub const STATE_TOPOLOGY_CENSUS_MAGIC: &[u8; 4] = b"H4SC";
pub const STATE_TOPOLOGY_CENSUS_VERSION: u16 = 1;
pub const STATE_TOPOLOGY_CENSUS_HEADER_LEN: usize = 16;

/// The most states one census may name.
///
/// NOT the structural id ceiling. `build_profile::MAX_STATE_ID` is
/// `i32::MAX - 1`, because the engine's state table length is a signed int and
/// this product refuses to hardcode a content-derived count. But the decoder
/// needs a cardinality ceiling BEFORE it builds anything, and a ceiling of two
/// billion is no ceiling: the id-set encodings compress, so a fifteen-byte
/// canonical block can declare a maximal run over the whole id space and ask
/// this decoder for 8 GiB.
///
/// So the bound is the codec's resource ceiling, which is itself derived from
/// what the recorder will capture rather than from what a map may contain —
/// see [`id_set_codec::MAX_DECODABLE_IDS`]. A state roster larger than the
/// recorder will walk is refused at capture time, loudly, instead of being
/// mis-decoded quietly here.
pub const STATE_TOPOLOGY_CENSUS_MAX_STATES: usize = MAX_DECODABLE_IDS;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTopologyCensusV1 {
    /// Every state id the map holds, sorted ascending, no duplicates,
    /// each in `1..=MAX_STATE_ID`. Never empty: a map with no states is a
    /// topology read failure, and a failed read publishes NO census rather
    /// than an empty one — absence of the record is the named unavailability.
    pub state_ids: Vec<u32>,
}

impl StateTopologyCensusV1 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.state_ids.is_empty() {
            return Err("state topology census has no states; publish nothing instead".into());
        }
        if !self.state_ids.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err("state topology census ids are not strictly ascending".into());
        }
        let first = *self.state_ids.first().expect("nonempty");
        let last = *self.state_ids.last().expect("nonempty");
        if first == 0 || last > crate::build_profile::MAX_STATE_ID {
            return Err(format!(
                "state topology census id out of range: {first}..={last} not within 1..={}",
                crate::build_profile::MAX_STATE_ID
            ));
        }
        let mut payload = Vec::with_capacity(STATE_TOPOLOGY_CENSUS_HEADER_LEN + 32);
        payload.extend_from_slice(STATE_TOPOLOGY_CENSUS_MAGIC);
        payload.extend_from_slice(&STATE_TOPOLOGY_CENSUS_VERSION.to_le_bytes());
        payload.extend_from_slice(&(STATE_TOPOLOGY_CENSUS_HEADER_LEN as u16).to_le_bytes());
        // flags: none defined; unknown bits are refused by the decoder.
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        encode_set_block(&mut payload, &self.state_ids)
            .map_err(|reason| format!("state topology census state set: {reason}"))?;
        Ok(payload)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.len() < STATE_TOPOLOGY_CENSUS_HEADER_LEN {
            return Err(format!(
                "state topology census payload is {} bytes, shorter than its {}-byte header",
                payload.len(),
                STATE_TOPOLOGY_CENSUS_HEADER_LEN
            ));
        }
        if &payload[0..4] != STATE_TOPOLOGY_CENSUS_MAGIC {
            return Err("state topology census magic mismatch".into());
        }
        let version = u16::from_le_bytes(payload[4..6].try_into().expect("2 bytes"));
        if version != STATE_TOPOLOGY_CENSUS_VERSION {
            return Err(format!(
                "state topology census version {version} is not {STATE_TOPOLOGY_CENSUS_VERSION}"
            ));
        }
        let header_len = u16::from_le_bytes(payload[6..8].try_into().expect("2 bytes")) as usize;
        if header_len != STATE_TOPOLOGY_CENSUS_HEADER_LEN {
            return Err(format!(
                "state topology census header length {header_len} is not {STATE_TOPOLOGY_CENSUS_HEADER_LEN}"
            ));
        }
        let flags = u32::from_le_bytes(payload[8..12].try_into().expect("4 bytes"));
        if flags != 0 {
            return Err(format!(
                "state topology census has unknown flag bits {flags:#010x}"
            ));
        }
        if u32::from_le_bytes(payload[12..16].try_into().expect("4 bytes")) != 0 {
            return Err("state topology census reserved field is nonzero".into());
        }
        let mut at = STATE_TOPOLOGY_CENSUS_HEADER_LEN;
        let state_ids = decode_set_block(payload, &mut at, STATE_TOPOLOGY_CENSUS_MAX_STATES)
            .map_err(|reason| format!("state topology census state set: {reason}"))?;
        if at != payload.len() {
            return Err(format!(
                "state topology census has {} trailing byte(s)",
                payload.len() - at
            ));
        }
        if state_ids.is_empty() {
            return Err("state topology census names no states".into());
        }
        let first = *state_ids.first().expect("nonempty");
        let last = *state_ids.last().expect("nonempty");
        if first == 0 || last > crate::build_profile::MAX_STATE_ID {
            return Err(format!(
                "state topology census id out of range: {first}..={last} not within 1..={}",
                crate::build_profile::MAX_STATE_ID
            ));
        }
        Ok(Self { state_ids })
    }
}

/// Where an `H4SC` may legally sit: at the very start of a logical stream, or
/// directly behind another PREAMBLE record.
///
/// This is deliberately STRICTER than the content-identity census's rule, which
/// admits any frame boundary (`FrameEnd`, `FrameRefused`) and so permits a
/// first publication arbitrarily late in a stream. The owner's ruling for this
/// record is not "at a frame boundary", it is "ONCE at the start of the
/// recording" (2026-08-20), and a validator that accepts record five hundred
/// does not encode that rule — it would let a producer regression ship a stream
/// whose early hours have no roster, and call it conforming.
///
/// The legal predecessors are exactly the two records that can precede it in a
/// preamble, which is the only place the producer publishes it: the install
/// freeze census and the content-identity census. That covers the attach
/// preamble and every rollover preamble, where a successor stream re-publishes
/// the same payload so each independently consumable stream carries its own
/// copy.
pub fn validate_state_topology_census_placement(
    ordinary_records_before: u64,
    previous_record_type: Option<KnownRecordType>,
    already_seen_in_this_stream: bool,
) -> Result<(), &'static str> {
    if already_seen_in_this_stream {
        return Err("state_topology_census_already_seen_in_this_stream");
    }
    if ordinary_records_before == 0 {
        return Ok(());
    }
    match previous_record_type {
        Some(KnownRecordType::FreezeCensus) | Some(KnownRecordType::ContentIdentityCensus) => {
            Ok(())
        }
        _ => Err("state_topology_census_is_not_in_a_stream_preamble"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn census(ids: &[u32]) -> StateTopologyCensusV1 {
        StateTopologyCensusV1 {
            state_ids: ids.to_vec(),
        }
    }

    #[test]
    fn round_trips_a_contiguous_map() {
        // kills: dropping the encode/decode symmetry, or the runs encoding of
        // the ordinary one-run contiguous state roster.
        let ids: Vec<u32> = (1..=1_081).collect();
        let payload = census(&ids).encode().expect("encodes");
        // One contiguous run: header + 9-byte block header + first(1) + len(2).
        assert!(payload.len() <= STATE_TOPOLOGY_CENSUS_HEADER_LEN + 9 + 8);
        let decoded = StateTopologyCensusV1::decode(&payload).expect("decodes");
        assert_eq!(decoded.state_ids, ids);
    }

    #[test]
    fn refuses_empty_unsorted_and_out_of_range() {
        // kills: publishing an empty census (absence of the record is the
        // named unavailability), an unsorted roster, or an id past the ceiling.
        assert!(census(&[]).encode().is_err());
        assert!(census(&[2, 1]).encode().is_err());
        assert!(census(&[0, 1]).encode().is_err());
        assert!(census(&[crate::build_profile::MAX_STATE_ID + 1])
            .encode()
            .is_err());
    }

    #[test]
    fn refuses_unknown_flags_reserved_bytes_and_trailing_bytes() {
        // kills: accepting a flag bit no decoder understands, a nonzero
        // reserved word, or bytes after the set block.
        let good = census(&[1, 2, 3]).encode().expect("encodes");
        let mut flags = good.clone();
        flags[8] = 1;
        assert!(StateTopologyCensusV1::decode(&flags)
            .unwrap_err()
            .contains("unknown flag bits"));
        let mut reserved = good.clone();
        reserved[12] = 1;
        assert!(StateTopologyCensusV1::decode(&reserved)
            .unwrap_err()
            .contains("reserved"));
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(StateTopologyCensusV1::decode(&trailing)
            .unwrap_err()
            .contains("trailing"));
        StateTopologyCensusV1::decode(&good).expect("the unmutated payload still decodes");
    }

    #[test]
    fn placement_admits_a_preamble_and_nothing_later() {
        // kills: letting a second census into one stream, admitting it mid-frame
        // where a reader cannot anchor it, or - the one this record needs and
        // the content-identity census does not enforce - admitting a FIRST
        // publication arbitrarily late. The owner's rule is "once at the START
        // of the recording", so a census behind a closed FRAME is not merely
        // unusual, it is a producer that did not follow the ruling.
        assert_eq!(
            validate_state_topology_census_placement(0, None, false),
            Ok(())
        );
        for preamble in [
            KnownRecordType::FreezeCensus,
            KnownRecordType::ContentIdentityCensus,
        ] {
            assert_eq!(
                validate_state_topology_census_placement(5, Some(preamble), false),
                Ok(()),
                "a rollover preamble re-publishes the roster behind {preamble:?}"
            );
        }
        for late in [
            KnownRecordType::FrameEnd,
            KnownRecordType::FrameRefused,
            KnownRecordType::DomainData,
        ] {
            assert!(
                validate_state_topology_census_placement(5, Some(late), false).is_err(),
                "{late:?} is not a preamble; a first roster there is too late"
            );
        }
        assert!(validate_state_topology_census_placement(0, None, true).is_err());
    }
}

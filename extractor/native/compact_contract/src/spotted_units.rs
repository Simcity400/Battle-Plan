//! Pointer-free exact-a729 current-POV map-spotted unit carrier (d95).

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 95;
pub const DOMAIN_NAME: &str = "pov_spotted_units_live";
pub const CARRIER_MAGIC: [u8; 4] = *b"SPU1";
pub const CARRIER_VERSION: u16 = 1;
/// A POV country id is a country-database id, so it carries the family bound:
/// [`crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID`]. Was `2_048`, a guess
/// at the vanilla-plus-DLC country count (vanilla defines 439 tags), which cost
/// this whole carrier on any overhaul whose human player sat above it.
pub const MAX_POV_COUNTRY_ID: u32 = crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_ROWS_PER_SURFACE: usize = 131_072;
pub const FLAG_LAND_FRAME: u16 = 1 << 0;
pub const FLAG_TASKFORCE_FRAME: u16 = 1 << 1;
/// The land frame carries the hour its PRODUCER observed at, not the hour the
/// collector drained it in.
///
/// # Why the drain hour is not enough
///
/// D95 retains a producer frame and re-serves it until the producer replaces
/// it, which is what stops a speed-5 game from opening false coverage holes on
/// every hour between two renderer calls. The consequence is that the hour
/// label on the record is the hour of the DRAIN, and the payload said nothing
/// about when the state it carries was observed -- so "the POV has seen these
/// units since H-1" and "the POV saw these units in H" were the same bytes.
/// Moving the drain to the front of the frame build widened the window in
/// which a producer publication lands after the drain from roughly 7 ms to
/// roughly 80 ms, which makes the distinction worth stating rather than
/// arguing.
///
/// The producer reads its hour from the SAME authenticated root the collector's
/// own date label comes from -- the CGameState singleton at
/// `build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA`, offset
/// `0x468` -- so the two words are directly comparable and their difference is
/// the frame's age in hours.
///
/// Additive, in the shape the D97 kind-3 change already set: the carrier
/// version does NOT move, the fact rides a previously unused flag bit, and a
/// payload without the bit decodes exactly as it did before. A decoder that
/// predates the bit refuses the payload rather than silently dropping the
/// field, which is the correct failure for a fact the fold reasons with.
pub const FLAG_LAND_FRAME_HOUR: u16 = 1 << 2;
/// The task-force frame's producer hour. See [`FLAG_LAND_FRAME_HOUR`].
pub const FLAG_TASKFORCE_FRAME_HOUR: u16 = 1 << 3;
const KNOWN_FLAGS: u16 =
    FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME | FLAG_LAND_FRAME_HOUR | FLAG_TASKFORCE_FRAME_HOUR;
const HEADER_LEN: usize = 44;
const ROW_LEN: usize = 12;
const HOUR_LEN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpottedUnitHandle {
    pub type_id: u32,
    pub stable_id: u32,
    pub raw_intel: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpottedUnitsSnapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    pub land_frame_sequence: Option<u64>,
    pub taskforce_frame_sequence: Option<u64>,
    /// The raw engine date word the LAND producer read at the moment it
    /// published, or `None` when the producer's clock read failed. See
    /// [`FLAG_LAND_FRAME_HOUR`].
    pub land_frame_hour_raw: Option<i32>,
    /// The task-force producer's own hour. See [`FLAG_LAND_FRAME_HOUR`].
    pub taskforce_frame_hour_raw: Option<i32>,
    pub land: Vec<SpottedUnitHandle>,
    pub taskforces: Vec<SpottedUnitHandle>,
}

impl SpottedUnitsSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.capture_sequence == 0 {
            return Err("spotted-units capture sequence is zero".into());
        }
        if self.pov_country_id == 0 || self.pov_country_id > MAX_POV_COUNTRY_ID {
            return Err("spotted-units POV country ID is outside the admitted range".into());
        }
        validate_surface("land", self.land_frame_sequence, &self.land, 250)?;
        validate_surface(
            "taskforce",
            self.taskforce_frame_sequence,
            &self.taskforces,
            150,
        )?;
        if self.land_frame_sequence.is_none() && self.taskforce_frame_sequence.is_none() {
            return Err("spotted-units snapshot has no complete producer frame".into());
        }
        // A producer hour is a statement BY a producer frame. Without the
        // frame there is nothing for it to date, and admitting one would let a
        // consumer age an observation that is not in the payload.
        if self.land_frame_hour_raw.is_some() && self.land_frame_sequence.is_none() {
            return Err("spotted-units land producer hour has no land frame".into());
        }
        if self.taskforce_frame_hour_raw.is_some() && self.taskforce_frame_sequence.is_none() {
            return Err("spotted-units taskforce producer hour has no taskforce frame".into());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut flags = 0u16;
        if self.land_frame_sequence.is_some() {
            flags |= FLAG_LAND_FRAME;
        }
        if self.taskforce_frame_sequence.is_some() {
            flags |= FLAG_TASKFORCE_FRAME;
        }
        if self.land_frame_hour_raw.is_some() {
            flags |= FLAG_LAND_FRAME_HOUR;
        }
        if self.taskforce_frame_hour_raw.is_some() {
            flags |= FLAG_TASKFORCE_FRAME_HOUR;
        }
        let land_count = u32::try_from(self.land.len()).map_err(|_| "land row count overflow")?;
        let taskforce_count =
            u32::try_from(self.taskforces.len()).map_err(|_| "taskforce row count overflow")?;
        let mut out = Vec::with_capacity(
            HEADER_LEN + ROW_LEN.saturating_mul(self.land.len() + self.taskforces.len()),
        );
        out.extend_from_slice(&CARRIER_MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&self.capture_sequence.to_le_bytes());
        out.extend_from_slice(&self.pov_country_id.to_le_bytes());
        out.extend_from_slice(&land_count.to_le_bytes());
        out.extend_from_slice(&taskforce_count.to_le_bytes());
        out.extend_from_slice(&self.land_frame_sequence.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&self.taskforce_frame_sequence.unwrap_or(0).to_le_bytes());
        for row in self.land.iter().chain(self.taskforces.iter()) {
            out.extend_from_slice(&row.type_id.to_le_bytes());
            out.extend_from_slice(&row.stable_id.to_le_bytes());
            out.push(row.raw_intel);
            out.extend_from_slice(&[0u8; 3]);
        }
        // The producer hours ride AFTER the rows, one 8-byte word per set flag,
        // in (land, taskforce) order. Appending rather than widening the header
        // is what keeps every byte offset the pre-existing decoders read at
        // exactly where it was.
        for hour in [self.land_frame_hour_raw, self.taskforce_frame_hour_raw]
            .into_iter()
            .flatten()
        {
            out.extend_from_slice(&i64::from(hour).to_le_bytes());
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || bytes[..4] != CARRIER_MAGIC {
            return Err("spotted-units carrier magic/length mismatch".into());
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if version != CARRIER_VERSION || flags & !KNOWN_FLAGS != 0 {
            return Err("spotted-units carrier version/flags mismatch".into());
        }
        let capture_sequence = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let pov_country_id = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let land_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        let taskforce_count = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
        let land_sequence = u64::from_le_bytes(bytes[28..36].try_into().unwrap());
        let taskforce_sequence = u64::from_le_bytes(bytes[36..44].try_into().unwrap());
        let row_count = land_count
            .checked_add(taskforce_count)
            .ok_or("spotted-units row count overflow")?;
        let hour_words = usize::from(flags & FLAG_LAND_FRAME_HOUR != 0)
            + usize::from(flags & FLAG_TASKFORCE_FRAME_HOUR != 0);
        let expected = HEADER_LEN
            .checked_add(
                ROW_LEN
                    .checked_mul(row_count)
                    .ok_or("spotted-units length overflow")?,
            )
            .and_then(|len| len.checked_add(HOUR_LEN.saturating_mul(hour_words)))
            .ok_or("spotted-units length overflow")?;
        if bytes.len() != expected {
            return Err("spotted-units carrier is truncated or has trailing bytes".into());
        }
        let mut rows = Vec::with_capacity(row_count);
        let mut cursor = 44;
        for _ in 0..row_count {
            let type_id = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap());
            let stable_id = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap());
            let raw_intel = bytes[cursor + 8];
            if bytes[cursor + 9..cursor + 12] != [0u8; 3] {
                return Err("spotted-units row reserved bytes are nonzero".into());
            }
            rows.push(SpottedUnitHandle {
                type_id,
                stable_id,
                raw_intel,
            });
            cursor += ROW_LEN;
        }
        let taskforces = rows.split_off(land_count);
        let mut hour = || {
            let word = i64::from_le_bytes(bytes[cursor..cursor + HOUR_LEN].try_into().unwrap());
            cursor += HOUR_LEN;
            i32::try_from(word).map_err(|_| {
                "spotted-units producer hour is outside the engine's date range".to_string()
            })
        };
        let land_frame_hour_raw = match flags & FLAG_LAND_FRAME_HOUR != 0 {
            true => Some(hour()?),
            false => None,
        };
        let taskforce_frame_hour_raw = match flags & FLAG_TASKFORCE_FRAME_HOUR != 0 {
            true => Some(hour()?),
            false => None,
        };
        let snapshot = Self {
            capture_sequence,
            pov_country_id,
            land_frame_sequence: (flags & FLAG_LAND_FRAME != 0).then_some(land_sequence),
            taskforce_frame_sequence: (flags & FLAG_TASKFORCE_FRAME != 0)
                .then_some(taskforce_sequence),
            land_frame_hour_raw,
            taskforce_frame_hour_raw,
            land: rows,
            taskforces,
        };
        if (flags & FLAG_LAND_FRAME == 0 && land_sequence != 0)
            || (flags & FLAG_TASKFORCE_FRAME == 0 && taskforce_sequence != 0)
        {
            return Err("spotted-units absent frame has a nonzero sequence".into());
        }
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SpottedUnitsSnapshot {
        SpottedUnitsSnapshot {
            capture_sequence: 9,
            pov_country_id: 12,
            land_frame_sequence: Some(4),
            taskforce_frame_sequence: Some(5),
            land_frame_hour_raw: None,
            taskforce_frame_hour_raw: None,
            land: vec![SpottedUnitHandle {
                type_id: 1,
                stable_id: 2,
                raw_intel: 250,
            }],
            taskforces: vec![SpottedUnitHandle {
                type_id: 3,
                stable_id: 4,
                raw_intel: 150,
            }],
        }
    }

    /// Law: the producer hour is ADDITIVE. A payload without it is byte-for-byte
    /// what this carrier produced before the field existed.
    ///
    /// The version does not move and the header does not widen, exactly as the
    /// D97 kind-3 change did it: the fact rides a previously unused flag bit and
    /// an 8-byte word AFTER the rows, so every offset a pre-existing reader
    /// walks is where it was. A capture archived before this lane still decodes.
    ///
    /// MUTATION RUN: write the hour words into the header instead, between
    /// `pov_country_id` and the row counts. OBSERVED: FAILED on the no-hour
    /// length -- every archived payload's row counts move by 16 bytes.
    #[test]
    fn a_payload_without_a_producer_hour_is_unchanged() {
        let bare = snapshot();
        let bytes = bare.encode().expect("the codec encodes this snapshot");
        assert_eq!(
            bytes.len(),
            HEADER_LEN + 2 * ROW_LEN,
            "a snapshot with no producer hour is exactly header plus rows"
        );
        assert_eq!(
            u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
            CARRIER_VERSION,
            "an additive field does not mint a new carrier version"
        );
        assert_eq!(
            u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
            FLAG_LAND_FRAME | FLAG_TASKFORCE_FRAME,
            "and sets no hour flag it does not carry"
        );
        assert_eq!(SpottedUnitsSnapshot::decode(&bytes).unwrap(), bare);
    }

    /// Law: each surface's producer hour round-trips independently.
    ///
    /// Independently, because the two producers publish on different engine
    /// calls and a drain routinely serves one fresh frame beside one retained
    /// one -- which is the whole reason the hour is on the wire.
    ///
    /// MUTATION RUN: swap the encode order of the two hour words while leaving
    /// the decode order alone. OBSERVED: FAILED on the both-hours case, land
    /// reading 4099 against 4100.
    #[test]
    fn each_surfaces_producer_hour_round_trips_on_its_own() {
        for (land_hour, taskforce_hour, extra_words) in [
            (Some(4_100), None, 1),
            (None, Some(4_099), 1),
            (Some(4_100), Some(4_099), 2),
            (Some(-7), Some(0), 2),
        ] {
            let mut value = snapshot();
            value.land_frame_hour_raw = land_hour;
            value.taskforce_frame_hour_raw = taskforce_hour;
            let bytes = value.encode().expect("the codec encodes this snapshot");
            assert_eq!(
                bytes.len(),
                HEADER_LEN + 2 * ROW_LEN + HOUR_LEN * extra_words,
                "one word per hour actually carried: {land_hour:?} {taskforce_hour:?}"
            );
            let decoded = SpottedUnitsSnapshot::decode(&bytes).expect("round trip");
            assert_eq!(decoded.land_frame_hour_raw, land_hour);
            assert_eq!(decoded.taskforce_frame_hour_raw, taskforce_hour);
            assert_eq!(decoded, value);
        }
    }

    /// Law: a producer hour needs the producer frame it dates.
    ///
    /// An hour without its frame would let a consumer age an observation that
    /// is not in the payload -- the same class of claim as a row without a
    /// complete frame, which `validate_surface` has always refused.
    ///
    /// MUTATION RUN: delete the two `_hour_raw.is_some() && _sequence.is_none()`
    /// arms from `validate`. OBSERVED: both encodes below succeed.
    #[test]
    fn a_producer_hour_may_not_outlive_its_frame() {
        let mut orphan_land = snapshot();
        orphan_land.land_frame_sequence = None;
        orphan_land.land = Vec::new();
        orphan_land.land_frame_hour_raw = Some(4_100);
        assert!(orphan_land.encode().is_err());

        let mut orphan_taskforce = snapshot();
        orphan_taskforce.taskforce_frame_sequence = None;
        orphan_taskforce.taskforces = Vec::new();
        orphan_taskforce.taskforce_frame_hour_raw = Some(4_100);
        assert!(orphan_taskforce.encode().is_err());
    }

    /// Law: a truncated or padded hour tail is refused, not read as a row.
    ///
    /// MUTATION RUN: drop the `hour_words` term from the expected length.
    /// OBSERVED: the truncations below decode, and the hour words are silently
    /// lost.
    #[test]
    fn a_truncated_producer_hour_tail_fails_closed() {
        let mut value = snapshot();
        value.land_frame_hour_raw = Some(4_100);
        value.taskforce_frame_hour_raw = Some(4_099);
        let bytes = value.encode().expect("the codec encodes this snapshot");
        for cut in 1..=HOUR_LEN {
            assert!(
                SpottedUnitsSnapshot::decode(&bytes[..bytes.len() - cut]).is_err(),
                "a tail short by {cut} bytes must not decode"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(SpottedUnitsSnapshot::decode(&extended).is_err());
    }
}

fn validate_surface(
    name: &str,
    sequence: Option<u64>,
    rows: &[SpottedUnitHandle],
    minimum_intel: u8,
) -> Result<(), String> {
    if rows.len() > MAX_ROWS_PER_SURFACE {
        return Err(format!(
            "spotted-units {name} row count exceeds the admitted ceiling"
        ));
    }
    match sequence {
        None if !rows.is_empty() => {
            return Err(format!("spotted-units {name} rows lack a complete frame"));
        }
        Some(0) => return Err(format!("spotted-units {name} frame sequence is zero")),
        _ => {}
    }
    let mut identities = BTreeSet::new();
    let mut prior = None;
    for row in rows {
        if row.type_id == 0 || row.stable_id == 0 || row.raw_intel < minimum_intel {
            return Err(format!(
                "spotted-units {name} row is outside the admitted range"
            ));
        }
        let key = (row.type_id, row.stable_id);
        if prior.is_some_and(|value| value >= key) || !identities.insert(key) {
            return Err(format!(
                "spotted-units {name} rows are not unique and sorted"
            ));
        }
        prior = Some(key);
    }
    Ok(())
}

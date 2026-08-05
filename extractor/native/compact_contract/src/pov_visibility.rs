//! Pointer-free exact-a729 current-POV province visibility carrier (d94).

pub const DOMAIN_ID: u16 = 94;
pub const DOMAIN_NAME: &str = "pov_province_visibility_live";
pub const CARRIER_MAGIC: [u8; 4] = *b"FOW1";
pub const CARRIER_VERSION: u16 = 1;
pub const MAX_PROVINCE_SLOTS: usize = 65_535;
pub const MAX_POV_COUNTRY_ID: u32 = 2_048;
/// The frame carries the hour its PRODUCER observed at, not the hour the
/// collector drained it in.
///
/// # Why the drain hour is not enough
///
/// D94 retains the last authenticated visibility vector and re-serves it until
/// the producer replaces it (`fow_visibility_live::a729::drain_latest`), which
/// is what stops a speed-5 game from opening false coverage holes on every hour
/// between two renderer calls. The consequence is that the hour label on the
/// record is the hour of the DRAIN, and the payload said nothing about when the
/// state it carries was observed -- so "this POV's fog has stood since H-1" and
/// "this POV's fog was rebuilt in H" were the same bytes. Moving the drain to
/// the front of the frame build (lane AO) widened the window in which a
/// producer publication lands after the drain, which makes the distinction
/// worth stating rather than arguing.
///
/// The producer reads its hour from the `CGameState` it is HANDED, at
/// `GS_CURRENT_DATE_OFFSET` (`0x468`) -- the same object and the same word the
/// collector's own frame label comes from. That the parameter is the
/// is already load-bearing in this lane: `read_pov_input` reads the
/// `+0x520`/`+0x524` pair off that same pointer to reconstruct the producer's
/// own rebuild arm. So the two words subtract, and their difference is the
/// frame's age in hours.
///
/// Additive, in the shape the D95 SPU1 change set: the carrier version does NOT
/// move, the fact rides the previously-reserved flag word, and a payload
/// without the bit decodes exactly as it did before. The word rides AFTER the
/// province vector, so every byte offset a pre-existing decoder reads is where
/// it was.
pub const FLAG_PRODUCER_HOUR: u16 = 1 << 0;
/// Every flag bit this carrier version defines. An unknown bit is refused, so
/// the strictness of the old `reserved != 0` check is preserved exactly: the
/// only word that decodes now and did not decode before is one that sets
/// [`FLAG_PRODUCER_HOUR`] and nothing else.
const KNOWN_FLAGS: u16 = FLAG_PRODUCER_HOUR;
pub const HEADER_LEN: usize = 24;
const HOUR_LEN: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PovVisibilitySnapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    /// The raw engine date word the producer read at the moment its invocation
    /// began, or `None` when that read failed. See [`FLAG_PRODUCER_HOUR`].
    pub producer_hour_raw: Option<i32>,
    /// Complete engine byte vector. Index is the province database ID.
    pub province_levels: Vec<u8>,
}

impl PovVisibilitySnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.capture_sequence == 0 {
            return Err("visibility capture sequence is zero".into());
        }
        if self.pov_country_id == 0 || self.pov_country_id > MAX_POV_COUNTRY_ID {
            return Err("visibility POV country ID is outside the admitted range".into());
        }
        if self.province_levels.is_empty() || self.province_levels.len() > MAX_PROVINCE_SLOTS {
            return Err("visibility province vector length is outside the admitted range".into());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let count = u32::try_from(self.province_levels.len())
            .map_err(|_| "visibility province vector length does not fit u32")?;
        let mut flags = 0u16;
        if self.producer_hour_raw.is_some() {
            flags |= FLAG_PRODUCER_HOUR;
        }
        let mut out = Vec::with_capacity(HEADER_LEN + self.province_levels.len() + HOUR_LEN);
        out.extend_from_slice(&CARRIER_MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&self.capture_sequence.to_le_bytes());
        out.extend_from_slice(&self.pov_country_id.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&self.province_levels);
        // The producer hour rides AFTER the province vector, one 8-byte word
        // when the flag is set and no bytes at all when it is not. Appending
        // rather than widening the header is what keeps every byte offset the
        // pre-existing decoders read at exactly where it was.
        if let Some(hour) = self.producer_hour_raw {
            out.extend_from_slice(&i64::from(hour).to_le_bytes());
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || bytes[..4] != CARRIER_MAGIC {
            return Err("visibility carrier magic/length mismatch".into());
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if version != CARRIER_VERSION || flags & !KNOWN_FLAGS != 0 {
            return Err("visibility carrier version/reserved mismatch".into());
        }
        let capture_sequence = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let pov_country_id = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        let hour_words = usize::from(flags & FLAG_PRODUCER_HOUR != 0);
        if bytes.len()
            != HEADER_LEN
                .checked_add(count)
                .and_then(|len| len.checked_add(HOUR_LEN.saturating_mul(hour_words)))
                .ok_or("visibility carrier length overflow")?
        {
            return Err("visibility carrier is truncated or has trailing bytes".into());
        }
        let producer_hour_raw = match flags & FLAG_PRODUCER_HOUR != 0 {
            true => {
                let at = HEADER_LEN + count;
                let word = i64::from_le_bytes(bytes[at..at + HOUR_LEN].try_into().unwrap());
                Some(i32::try_from(word).map_err(|_| {
                    "visibility producer hour is outside the engine's date range".to_string()
                })?)
            }
            false => None,
        };
        let snapshot = Self {
            capture_sequence,
            pov_country_id,
            producer_hour_raw,
            province_levels: bytes[HEADER_LEN..HEADER_LEN + count].to_vec(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> PovVisibilitySnapshot {
        PovVisibilitySnapshot {
            capture_sequence: 9,
            pov_country_id: 12,
            producer_hour_raw: None,
            province_levels: vec![0, 1, 2, 3],
        }
    }

    /// Law: the producer hour is ADDITIVE. A payload without it is byte-for-byte
    /// what this carrier produced before the field existed.
    ///
    /// The version does not move and the header does not widen, exactly as the
    /// D95 SPU1 change did it: the fact rides the previously-reserved flag word
    /// and an 8-byte word AFTER the province vector, so every offset a
    /// pre-existing reader walks is where it was. A capture archived before this
    /// lane still decodes, and this test states the archived bytes literally
    /// rather than deriving them from the encoder it is checking.
    ///
    /// MUTATION RUN: write the hour word into the payload unconditionally,
    /// before the province vector. OBSERVED: FAILED -- `assertion `left ==
    /// right` failed: a snapshot with no producer hour is exactly header plus
    /// provinces / left: 36 / right: 28`, taking
    /// `a_producer_hour_round_trips_in_exactly_one_appended_word` and
    /// `a_truncated_or_padded_producer_hour_tail_fails_closed` with it.
    #[test]
    fn a_payload_without_a_producer_hour_is_unchanged() {
        let bare = snapshot();
        let bytes = bare.encode().expect("the codec encodes this snapshot");
        assert_eq!(
            bytes.len(),
            HEADER_LEN + 4,
            "a snapshot with no producer hour is exactly header plus provinces"
        );
        // The literal bytes a recorder built before this field existed.
        let archived: Vec<u8> = b"FOW1"
            .iter()
            .copied()
            .chain(1u16.to_le_bytes())
            .chain(0u16.to_le_bytes())
            .chain(9u64.to_le_bytes())
            .chain(12u32.to_le_bytes())
            .chain(4u32.to_le_bytes())
            .chain([0u8, 1, 2, 3])
            .collect();
        assert_eq!(bytes, archived, "the encoder still emits the archived shape");
        assert_eq!(
            PovVisibilitySnapshot::decode(&archived).expect("archived payloads still decode"),
            bare
        );
    }

    /// Law: the producer hour round-trips, and costs exactly one word.
    ///
    /// MUTATION RUN: drop the `hour_words` term from the expected length in
    /// `decode`. OBSERVED: FAILED -- `round trip: "visibility carrier is
    /// truncated or has trailing bytes"` on the first hour-carrying arm, and
    /// `a_truncated_or_padded_producer_hour_tail_fails_closed` panics with
    /// `range end index 36 out of range for slice of length 28` -- the length
    /// equation is what makes the hour-word slice in-bounds, so dropping its
    /// term does not merely lose the field, it un-bounds the read.
    #[test]
    fn a_producer_hour_round_trips_in_exactly_one_appended_word() {
        for hour in [Some(60_832_312), Some(0), Some(-7), Some(i32::MIN), None] {
            let mut value = snapshot();
            value.producer_hour_raw = hour;
            let bytes = value.encode().expect("the codec encodes this snapshot");
            assert_eq!(
                bytes.len(),
                HEADER_LEN + 4 + HOUR_LEN * usize::from(hour.is_some()),
                "one word per hour actually carried: {hour:?}"
            );
            assert_eq!(
                u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
                if hour.is_some() { FLAG_PRODUCER_HOUR } else { 0 },
                "the flag word says exactly what the tail carries"
            );
            assert_eq!(
                u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
                CARRIER_VERSION,
                "an additive field does not mint a new carrier version"
            );
            let decoded = PovVisibilitySnapshot::decode(&bytes).expect("round trip");
            assert_eq!(decoded.producer_hour_raw, hour);
            assert_eq!(decoded, value);
        }
    }

    /// Law: the flag word is still refused for anything this version does not
    /// define, so widening `reserved` into `flags` gave up no strictness.
    ///
    /// A decoder that predates the bit refuses the payload rather than silently
    /// dropping the field, which is the correct failure for a fact a consumer
    /// reasons with -- and symmetrically, a decoder that postdates it must
    /// refuse a bit it cannot interpret rather than ignore the tail it implies.
    ///
    /// MUTATION RUN: replace `flags & !KNOWN_FLAGS != 0` with `false`.
    /// OBSERVED: FAILED -- `panicked at compact_contract\src\pov_visibility.rs:
    /// a reserved flag bit 1 must still be refused`.
    #[test]
    fn an_undefined_flag_bit_is_still_refused() {
        let bytes = snapshot().encode().expect("the codec encodes this snapshot");
        for bit in 1..16u32 {
            let mut dirty = bytes.clone();
            dirty[6..8].copy_from_slice(&(1u16 << bit).to_le_bytes());
            assert!(
                PovVisibilitySnapshot::decode(&dirty).is_err(),
                "a reserved flag bit {bit} must still be refused"
            );
        }
    }

    /// Law: a truncated or padded hour tail is refused, never read as a
    /// province byte.
    ///
    /// This is the d94 shape of D95's orphan-hour guard. D95 has two surfaces
    /// and either may be absent, so an hour there can outlive the frame it
    /// dates and `validate` refuses that pairing. D94 has exactly one surface
    /// and `validate` already requires a non-empty province vector, so an
    /// orphan hour is unconstructible -- the only way the word and the vector
    /// can disagree is a length disagreement, and that is what this pins.
    ///
    /// MUTATION RUN: slice the body as `bytes[HEADER_LEN..]` again instead of
    /// `bytes[HEADER_LEN..HEADER_LEN + count]`. OBSERVED: FAILED --
    /// `left: [0, 1, 2, 3, 56, 58, 160, 3, 0, 0, 0, 0] / right: [0, 1, 2, 3]`,
    /// i.e. the eight hour bytes are decoded as eight extra provinces (and the
    /// round-trip test above fails on the same eight bytes).
    #[test]
    fn a_truncated_or_padded_producer_hour_tail_fails_closed() {
        let mut value = snapshot();
        value.producer_hour_raw = Some(60_832_312);
        let bytes = value.encode().expect("the codec encodes this snapshot");
        for cut in 1..=HOUR_LEN {
            assert!(
                PovVisibilitySnapshot::decode(&bytes[..bytes.len() - cut]).is_err(),
                "a tail short by {cut} bytes must not decode"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(PovVisibilitySnapshot::decode(&extended).is_err());
        // And the hour word is not counted as provinces.
        assert_eq!(
            PovVisibilitySnapshot::decode(&bytes)
                .expect("round trip")
                .province_levels,
            vec![0u8, 1, 2, 3]
        );
    }

    /// Law: an hour word outside the engine's `int` date range is refused
    /// rather than truncated into one.
    ///
    /// MUTATION RUN: replace the `i32::try_from` with `word as i32`. OBSERVED:
    /// FAILED -- `assertion failed: PovVisibilitySnapshot::decode(&bytes)
    /// .is_err()`; the payload decodes and reports hour 0 for a word of
    /// 0x1_0000_0000.
    #[test]
    fn a_producer_hour_outside_the_engines_date_range_is_refused() {
        let mut value = snapshot();
        value.producer_hour_raw = Some(1);
        let mut bytes = value.encode().expect("the codec encodes this snapshot");
        let at = bytes.len() - HOUR_LEN;
        bytes[at..].copy_from_slice(&(1i64 << 32).to_le_bytes());
        assert!(PovVisibilitySnapshot::decode(&bytes).is_err());
    }
}

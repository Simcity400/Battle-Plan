pub const DOMAIN_ID: u16 = 94;
pub const DOMAIN_NAME: &str = "pov_province_visibility_live";
pub const CARRIER_MAGIC: [u8; 4] = *b"FOW1";
pub const CARRIER_VERSION: u16 = 1;
pub const MAX_PROVINCE_SLOTS: usize = 65_535;

pub const MAX_PROVINCE_SLOT_INDEX: u32 = (MAX_PROVINCE_SLOTS - 1) as u32;
pub const MAX_POV_COUNTRY_ID: u32 = crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const FLAG_PRODUCER_HOUR: u16 = 1 << 0;
const KNOWN_FLAGS: u16 = FLAG_PRODUCER_HOUR;
pub const HEADER_LEN: usize = 24;
const HOUR_LEN: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PovVisibilitySnapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    pub producer_hour_raw: Option<i32>,
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

    #[test]
    fn a_payload_without_a_producer_hour_is_unchanged() {
        let bare = snapshot();
        let bytes = bare.encode().expect("the codec encodes this snapshot");
        assert_eq!(
            bytes.len(),
            HEADER_LEN + 4,
            "a snapshot with no producer hour is exactly header plus provinces"
        );
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
        assert_eq!(
            bytes, archived,
            "the encoder still emits the archived shape"
        );
        assert_eq!(
            PovVisibilitySnapshot::decode(&archived).expect("archived payloads still decode"),
            bare
        );
    }

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
                if hour.is_some() {
                    FLAG_PRODUCER_HOUR
                } else {
                    0
                },
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

    #[test]
    fn an_undefined_flag_bit_is_still_refused() {
        let bytes = snapshot()
            .encode()
            .expect("the codec encodes this snapshot");
        for bit in 1..16u32 {
            let mut dirty = bytes.clone();
            dirty[6..8].copy_from_slice(&(1u16 << bit).to_le_bytes());
            assert!(
                PovVisibilitySnapshot::decode(&dirty).is_err(),
                "a reserved flag bit {bit} must still be refused"
            );
        }
    }

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
        assert_eq!(
            PovVisibilitySnapshot::decode(&bytes)
                .expect("round trip")
                .province_levels,
            vec![0u8, 1, 2, 3]
        );
    }

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

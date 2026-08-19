pub const CLOCK_UNREADABLE_INTERVAL_MAGIC: &[u8; 4] = b"H4CU";
pub const CLOCK_UNREADABLE_INTERVAL_VERSION: u16 = 1;
pub const CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockUnreadableIntervalRecordV1 {
    pub range_start_hour_raw: i32,
    pub range_end_hour_raw: i32,
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
}

impl ClockUnreadableIntervalRecordV1 {
    pub fn hour_count(&self) -> u64 {
        u64::try_from(i64::from(self.range_end_hour_raw) - i64::from(self.range_start_hour_raw) + 1)
            .unwrap_or(0)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut payload = vec![0u8; CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN];
        payload[..4].copy_from_slice(CLOCK_UNREADABLE_INTERVAL_MAGIC);
        payload[4..6].copy_from_slice(&CLOCK_UNREADABLE_INTERVAL_VERSION.to_le_bytes());
        payload[6..8]
            .copy_from_slice(&(CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN as u16).to_le_bytes());
        payload[12..16].copy_from_slice(&self.range_start_hour_raw.to_le_bytes());
        payload[16..20].copy_from_slice(&self.range_end_hour_raw.to_le_bytes());
        payload[20..24].copy_from_slice(&self.last_observed_hour_raw.to_le_bytes());
        payload[24..28].copy_from_slice(&self.resumed_hour_raw.to_le_bytes());
        payload[28..32].copy_from_slice(&self.unreadable_events.to_le_bytes());
        validate_clock_unreadable_interval_payload(&payload)?;
        Ok(payload)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_clock_unreadable_interval_payload(payload)?;
        let read_i32 = |start: usize| -> i32 {
            i32::from_le_bytes(
                payload[start..start + 4]
                    .try_into()
                    .expect("validated H4CU slice"),
            )
        };
        Ok(Self {
            range_start_hour_raw: read_i32(12),
            range_end_hour_raw: read_i32(16),
            last_observed_hour_raw: read_i32(20),
            resumed_hour_raw: read_i32(24),
            unreadable_events: u32::from_le_bytes(
                payload[28..32]
                    .try_into()
                    .expect("validated H4CU events slice"),
            ),
        })
    }
}

pub fn validate_clock_unreadable_interval_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() != CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL payload length {} is not \
             {CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    if &payload[..4] != CLOCK_UNREADABLE_INTERVAL_MAGIC {
        return Err("CLOCK_UNREADABLE_INTERVAL payload magic is invalid".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != CLOCK_UNREADABLE_INTERVAL_VERSION {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL payload version {version} is not \
             {CLOCK_UNREADABLE_INTERVAL_VERSION}"
        ));
    }
    let declared_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
    if declared_len != CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL declared length {declared_len} is not \
             {CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN}"
        ));
    }
    if u32::from_le_bytes(payload[8..12].try_into().expect("fixed reserved slice")) != 0 {
        return Err("CLOCK_UNREADABLE_INTERVAL reserved field is nonzero".to_string());
    }
    let read_i32 = |start: usize| -> i32 {
        i32::from_le_bytes(payload[start..start + 4].try_into().expect("fixed slice"))
    };
    let range_start = read_i32(12);
    let range_end = read_i32(16);
    let last_observed = read_i32(20);
    let resumed = read_i32(24);
    let unreadable_events =
        u32::from_le_bytes(payload[28..32].try_into().expect("fixed events slice"));
    for (name, hour) in [
        ("range_start", range_start),
        ("range_end", range_end),
        ("last_observed", last_observed),
        ("resumed", resumed),
    ] {
        if crate::calendar::hour_index_from_raw(hour).is_none() {
            return Err(format!(
                "CLOCK_UNREADABLE_INTERVAL {name} hour {hour} is invalid"
            ));
        }
    }
    if range_start > range_end {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL range {range_start}..={range_end} is empty"
        ));
    }
    if last_observed.checked_add(1) != Some(range_start) {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL range starts at {range_start}, not immediately after the \
             last observed hour {last_observed}"
        ));
    }
    if resumed.checked_sub(1) != Some(range_end) {
        return Err(format!(
            "CLOCK_UNREADABLE_INTERVAL range ends at {range_end}, not immediately before the \
             resumed hour {resumed}"
        ));
    }
    if unreadable_events == 0 {
        return Err(
            "CLOCK_UNREADABLE_INTERVAL claims an outage with no failed clock read".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::DATE_EPOCH_CONST;

    fn hour(offset: i32) -> i32 {
        DATE_EPOCH_CONST + 1936 * 365 * 24 + offset
    }

    fn record() -> ClockUnreadableIntervalRecordV1 {
        ClockUnreadableIntervalRecordV1 {
            range_start_hour_raw: hour(101),
            range_end_hour_raw: hour(104),
            last_observed_hour_raw: hour(100),
            resumed_hour_raw: hour(105),
            unreadable_events: 2,
        }
    }

    #[test]
    fn a_canonical_interval_round_trips() {
        let original = record();
        let payload = original.encode().expect("canonical interval encodes");
        assert_eq!(payload.len(), CLOCK_UNREADABLE_INTERVAL_PAYLOAD_LEN);
        assert_eq!(
            ClockUnreadableIntervalRecordV1::decode(&payload).expect("decodes"),
            original
        );
        assert_eq!(original.hour_count(), 4);
    }

    #[test]
    fn the_witnesses_must_prove_each_other() {
        let mut detached = record();
        detached.last_observed_hour_raw = hour(90);
        let error = detached.encode().expect_err("a detached start must refuse");
        assert!(
            error.contains("not immediately after the last observed hour"),
            "{error}"
        );

        let mut overrun = record();
        overrun.resumed_hour_raw = hour(200);
        let error = overrun.encode().expect_err("a detached end must refuse");
        assert!(
            error.contains("not immediately before the resumed hour"),
            "{error}"
        );

        let mut unwitnessed = record();
        unwitnessed.unreadable_events = 0;
        let error = unwitnessed
            .encode()
            .expect_err("an unwitnessed outage must refuse");
        assert!(error.contains("no failed clock read"), "{error}");

        let empty = ClockUnreadableIntervalRecordV1 {
            range_start_hour_raw: hour(101),
            range_end_hour_raw: hour(100),
            last_observed_hour_raw: hour(100),
            resumed_hour_raw: hour(101),
            unreadable_events: 1,
        };
        let error = empty.encode().expect_err("an empty interval must refuse");
        assert!(error.contains("is empty"), "{error}");
    }

    #[test]
    fn a_malformed_payload_is_refused_rather_than_guessed() {
        let good = record().encode().expect("encodes");

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(ClockUnreadableIntervalRecordV1::decode(&bad_magic).is_err());

        let mut bad_version = good.clone();
        bad_version[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(ClockUnreadableIntervalRecordV1::decode(&bad_version).is_err());

        let mut bad_reserved = good.clone();
        bad_reserved[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(ClockUnreadableIntervalRecordV1::decode(&bad_reserved).is_err());

        assert!(ClockUnreadableIntervalRecordV1::decode(&good[..31]).is_err());

        let mut invalid_hour = good;
        invalid_hour[12..16].copy_from_slice(&i32::MIN.to_le_bytes());
        assert!(ClockUnreadableIntervalRecordV1::decode(&invalid_hour).is_err());
    }
}

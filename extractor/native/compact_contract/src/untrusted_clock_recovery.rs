pub const UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS: i64 = 24 * 32;

pub const UNTRUSTED_CLOCK_RECOVERY_MAGIC: &[u8; 4] = b"H4CJ";
pub const UNTRUSTED_CLOCK_RECOVERY_VERSION: u16 = 1;
pub const UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UntrustedClockRecoveryRecordV1 {
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
    pub observed_delta_hours: i64,
    pub trust_ceiling_hours: i64,
}

impl UntrustedClockRecoveryRecordV1 {
    pub fn implied_unobserved_hours(&self) -> i64 {
        self.observed_delta_hours.saturating_sub(1).max(0)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let mut payload = vec![0u8; UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN];
        payload[..4].copy_from_slice(UNTRUSTED_CLOCK_RECOVERY_MAGIC);
        payload[4..6].copy_from_slice(&UNTRUSTED_CLOCK_RECOVERY_VERSION.to_le_bytes());
        payload[6..8].copy_from_slice(&(UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN as u16).to_le_bytes());
        payload[12..16].copy_from_slice(&self.last_observed_hour_raw.to_le_bytes());
        payload[16..20].copy_from_slice(&self.resumed_hour_raw.to_le_bytes());
        payload[20..24].copy_from_slice(&self.unreadable_events.to_le_bytes());
        payload[28..36].copy_from_slice(&self.observed_delta_hours.to_le_bytes());
        payload[36..44].copy_from_slice(&self.trust_ceiling_hours.to_le_bytes());
        validate_untrusted_clock_recovery_payload(&payload)?;
        Ok(payload)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_untrusted_clock_recovery_payload(payload)?;
        let read_i32 = |start: usize| -> i32 {
            i32::from_le_bytes(
                payload[start..start + 4]
                    .try_into()
                    .expect("validated H4CJ i32 slice"),
            )
        };
        let read_i64 = |start: usize| -> i64 {
            i64::from_le_bytes(
                payload[start..start + 8]
                    .try_into()
                    .expect("validated H4CJ i64 slice"),
            )
        };
        Ok(Self {
            last_observed_hour_raw: read_i32(12),
            resumed_hour_raw: read_i32(16),
            unreadable_events: u32::from_le_bytes(
                payload[20..24]
                    .try_into()
                    .expect("validated H4CJ events slice"),
            ),
            observed_delta_hours: read_i64(28),
            trust_ceiling_hours: read_i64(36),
        })
    }
}

pub fn validate_untrusted_clock_recovery_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() != UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY payload length {} is not \
             {UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    if &payload[..4] != UNTRUSTED_CLOCK_RECOVERY_MAGIC {
        return Err("UNTRUSTED_CLOCK_RECOVERY payload magic is invalid".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != UNTRUSTED_CLOCK_RECOVERY_VERSION {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY payload version {version} is not \
             {UNTRUSTED_CLOCK_RECOVERY_VERSION}"
        ));
    }
    let declared_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
    if declared_len != UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY declared length {declared_len} is not \
             {UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN}"
        ));
    }
    for (name, start) in [
        ("reserved", 8usize),
        ("reserved_mid", 24),
        ("reserved_tail", 44),
    ] {
        if u32::from_le_bytes(payload[start..start + 4].try_into().expect("fixed slice")) != 0 {
            return Err(format!("UNTRUSTED_CLOCK_RECOVERY {name} field is nonzero"));
        }
    }
    let read_i32 = |start: usize| -> i32 {
        i32::from_le_bytes(payload[start..start + 4].try_into().expect("fixed slice"))
    };
    let read_i64 = |start: usize| -> i64 {
        i64::from_le_bytes(payload[start..start + 8].try_into().expect("fixed slice"))
    };
    let last_observed = read_i32(12);
    let resumed = read_i32(16);
    let unreadable_events =
        u32::from_le_bytes(payload[20..24].try_into().expect("fixed events slice"));
    let observed_delta = read_i64(28);
    let trust_ceiling = read_i64(36);
    for (name, hour) in [("last_observed", last_observed), ("resumed", resumed)] {
        if crate::calendar::hour_index_from_raw(hour).is_none() {
            return Err(format!(
                "UNTRUSTED_CLOCK_RECOVERY {name} hour {hour} is invalid"
            ));
        }
    }
    if trust_ceiling != UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY trust ceiling {trust_ceiling} is not the pinned              {UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS}"
        ));
    }
    let actual_delta = i64::from(resumed) - i64::from(last_observed);
    if observed_delta != actual_delta {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY declares a delta of {observed_delta} hours, but resuming at \
             {resumed} from {last_observed} is a jump of {actual_delta}"
        ));
    }
    if observed_delta <= trust_ceiling {
        return Err(format!(
            "UNTRUSTED_CLOCK_RECOVERY delta {observed_delta} does not exceed the trust ceiling \
             {trust_ceiling}, so the recovery was trusted and belongs in a \
             CLOCK_UNREADABLE_INTERVAL"
        ));
    }
    if unreadable_events == 0 {
        return Err(
            "UNTRUSTED_CLOCK_RECOVERY claims a clock outage with no failed clock read".to_string(),
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

    fn record() -> UntrustedClockRecoveryRecordV1 {
        UntrustedClockRecoveryRecordV1 {
            last_observed_hour_raw: hour(100),
            resumed_hour_raw: hour(100 + 800),
            unreadable_events: 3,
            observed_delta_hours: 800,
            trust_ceiling_hours: UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS,
        }
    }

    #[test]
    fn a_canonical_untrusted_recovery_round_trips() {
        let original = record();
        let payload = original.encode().expect("canonical recovery encodes");
        assert_eq!(payload.len(), UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN);
        assert_eq!(
            UntrustedClockRecoveryRecordV1::decode(&payload).expect("decodes"),
            original
        );
        assert_eq!(original.implied_unobserved_hours(), 799);
    }

    #[test]
    fn the_declared_delta_must_be_the_observed_jump() {
        let mut understated = record();
        understated.observed_delta_hours = 769;
        let error = understated
            .encode()
            .expect_err("an understated delta must refuse");
        assert!(error.contains("is a jump of 800"), "{error}");

        let mut overstated = record();
        overstated.observed_delta_hours = 100_000;
        let error = overstated
            .encode()
            .expect_err("an overstated delta must refuse");
        assert!(error.contains("is a jump of 800"), "{error}");
    }

    #[test]
    fn a_trusted_recovery_may_not_wear_this_records_name() {
        let mut inside = record();
        inside.resumed_hour_raw = hour(100 + 768);
        inside.observed_delta_hours = 768;
        let error = inside
            .encode()
            .expect_err("a delta at the ceiling must refuse");
        assert!(
            error.contains("does not exceed the trust ceiling"),
            "{error}"
        );

        let mut lowered = record();
        lowered.last_observed_hour_raw = hour(100);
        lowered.resumed_hour_raw = hour(200);
        lowered.observed_delta_hours = 100;
        lowered.trust_ceiling_hours = 99;
        let error = lowered.encode().expect_err("a lowered ceiling must refuse");
        assert!(error.contains("is not the pinned"), "{error}");

        let mut zero_ceiling = record();
        zero_ceiling.trust_ceiling_hours = 0;
        let error = zero_ceiling
            .encode()
            .expect_err("a zero ceiling must refuse");
        assert!(error.contains("is not the pinned"), "{error}");
    }

    #[test]
    fn an_unwitnessed_outage_is_refused() {
        let mut unwitnessed = record();
        unwitnessed.unreadable_events = 0;
        let error = unwitnessed
            .encode()
            .expect_err("an unwitnessed outage must refuse");
        assert!(error.contains("no failed clock read"), "{error}");
    }

    #[test]
    fn a_malformed_payload_is_refused_rather_than_guessed() {
        let good = record().encode().expect("encodes");

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(UntrustedClockRecoveryRecordV1::decode(&bad_magic).is_err());

        let mut bad_version = good.clone();
        bad_version[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(UntrustedClockRecoveryRecordV1::decode(&bad_version).is_err());

        for start in [8usize, 24, 44] {
            let mut bad_reserved = good.clone();
            bad_reserved[start..start + 4].copy_from_slice(&1u32.to_le_bytes());
            assert!(
                UntrustedClockRecoveryRecordV1::decode(&bad_reserved).is_err(),
                "reserved field at {start} was accepted nonzero"
            );
        }

        assert!(UntrustedClockRecoveryRecordV1::decode(&good[..47]).is_err());

        let mut invalid_hour = good;
        invalid_hour[12..16].copy_from_slice(&i32::MIN.to_le_bytes());
        assert!(UntrustedClockRecoveryRecordV1::decode(&invalid_hour).is_err());
    }
}

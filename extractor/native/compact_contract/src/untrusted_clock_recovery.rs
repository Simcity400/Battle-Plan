//! Untrusted clock-recovery record (`H4CJ`).
//!
//! # The hole this closes
//!
//! The scheduler applies one ceiling to forward clock motion. On the ORDINARY
//! path a forward delta larger than that ceiling is a counter read it does not
//! believe: the observation is refused outright, the anomaly is counted, and
//! scheduling ends.
//!
//! On the RECOVERY path — the first stable observation after the authenticated
//! clock could not be read — the same untrusted value used to be accepted in
//! silence. It scheduled the resumed hour, it left no terminal, it incremented
//! no anomaly counter, and (once `H4CU` existed) its interval was refused
//! without saying which of two very different reasons refused it. A capture
//! could therefore publish hours whose entire hour labelling descended from a
//! clock read the producer itself would have rejected one branch over, and
//! nothing on the wire said so.
//!
//! The owner's ruling (2026-08-05) is that such a jump is ACCEPTED and RECORDED
//! — not made terminal. One failed clock read is far too weak a reason to
//! destroy a capture, and a partial capture still seals. So the resumed hour is
//! still scheduled, and this record is what makes that decision visible.
//!
//! # What this record says, and what it refuses to say
//!
//! It says: the clock could not be read for `unreadable_events` attempts after
//! hour `last_observed_hour`; it read again at `resumed_hour`; the two are
//! `observed_delta_hours` apart, which exceeds the producer's own trust ceiling
//! of `trust_ceiling_hours`; the producer therefore does NOT vouch for that
//! span, and every hour it publishes from `resumed_hour` onward rests on that
//! untrusted read.
//!
//! It does NOT say those hours elapsed. That is the whole difference from
//! [`crate::clock_unreadable_interval`] (`H4CU`), and the reason this is a
//! separate record rather than a flag on that one:
//!
//! * `H4CU` names a range and ACCOUNTS for it — the producer vouches that those
//!   hours elapsed unobserved, and a consumer may retire them from its
//!   unexplained set.
//! * `H4CJ` names the same two endpoints and accounts for NOTHING. The span
//!   rests entirely on a value the producer has just declared untrustworthy, so
//!   turning it into an accounted hour count would buy, with one failed clock
//!   read, a claim the ordinary path refuses outright.
//!
//! The two are minted by disjoint branches of one decision and can never both
//! describe the same outage: `observed_delta <= trust_ceiling` mints `H4CU`,
//! `observed_delta > trust_ceiling` mints this, and an outage that resumes in
//! the same hour or the next one loses no hour and mints neither. This record's
//! own grammar enforces its half of that split — `observed_delta_hours` must
//! exceed `trust_ceiling_hours`, so a producer cannot relabel an ordinary
//! outage as untrusted, and cannot use this record to escape naming an interval
//! it could have named.
//!
//! # Self-proving witnesses
//!
//! Every field is checked against the others, so a consumer never takes the
//! producer's word for the shape:
//!
//! * `observed_delta_hours == resumed_hour - last_observed_hour` — the delta is
//!   the ACTUAL jump size, recomputed from the two endpoints rather than
//!   asserted, so it cannot be understated into plausibility;
//! * `observed_delta_hours > trust_ceiling_hours` — a jump inside the ceiling is
//!   an ordinary outage and belongs in an `H4CU`, not here;
//! * `trust_ceiling_hours == UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS` — the
//!   ceiling is pinned, not declared, so a producer cannot manufacture
//!   untrustworthiness by lowering it and thereby escape the accountable `H4CU`
//!   branch for an outage it could have vouched for;
//! * `unreadable_events > 0` — this record only ever arises out of a clock
//!   outage, and a claim of one must be backed by at least one failed read.
//!   Zero would let an ordinary forward jump wear this record's name;
//! * both hours are valid calendar hours.
//!
//! Layout, little-endian:
//!
//! ```text
//! magic               [4]  b"H4CJ"
//! version             u16  = UNTRUSTED_CLOCK_RECOVERY_VERSION
//! declared_len        u16  = UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN
//! reserved            u32  MUST BE ZERO
//! last_observed_hour  i32
//! resumed_hour        i32
//! unreadable_events   u32  NON-ZERO
//! reserved_mid        u32  MUST BE ZERO
//! observed_delta      i64  == resumed_hour - last_observed_hour, > ceiling
//! trust_ceiling       i64  == UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS
//! reserved_tail       u32  MUST BE ZERO
//! -- 48 bytes --
//! ```

/// The ONE trust ceiling this record may claim, in hours.
///
/// Carried in the payload so the record substantiates its own classification --
/// and PINNED here so the claim cannot be manufactured. A producer free to
/// declare its own ceiling could set it to 1, call any two-hour outage
/// untrusted, and escape the accountable `H4CU` branch entirely: the hours
/// would leave the host's accounting with no vouched statement behind them and
/// the capture would look cleaner than it was. Requiring the shipping value
/// makes "untrusted" a fact about the delta rather than a choice.
///
/// `read_only_scheduler::MAX_FORWARD_DELTA` is the producer side of the same
/// constant and is checked against this by
/// `the_scheduler_ceiling_is_the_contract_ceiling`.
pub const UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS: i64 = 24 * 32;

pub const UNTRUSTED_CLOCK_RECOVERY_MAGIC: &[u8; 4] = b"H4CJ";
pub const UNTRUSTED_CLOCK_RECOVERY_VERSION: u16 = 1;
pub const UNTRUSTED_CLOCK_RECOVERY_PAYLOAD_LEN: usize = 48;

/// One published untrusted clock recovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UntrustedClockRecoveryRecordV1 {
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
    /// The ACTUAL jump size in hours. Recorded as the observed value, not as a
    /// boolean "too far": a consumer deciding what to do with the capture, and
    /// a human reading the manifest, both need to know whether the clock moved
    /// a day past the ceiling or a decade.
    pub observed_delta_hours: i64,
    /// The ceiling that judged the delta untrusted. Carried so the record
    /// substantiates its own claim instead of asking the consumer to know the
    /// producer's constant — and validated against
    /// [`UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS`], so carrying it is not
    /// the same as letting the producer choose it.
    pub trust_ceiling_hours: i64,
}

impl UntrustedClockRecoveryRecordV1 {
    /// How many hours lie strictly between the two endpoints.
    ///
    /// Deliberately NOT called `hour_count` and deliberately not comparable to
    /// [`crate::clock_unreadable_interval::ClockUnreadableIntervalRecordV1::hour_count`]:
    /// that one counts hours the producer VOUCHES elapsed, this one reports the
    /// width the untrusted read would IMPLY. Nothing may account for these
    /// hours on the strength of this number.
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
        // Encode by validating what was just written, so a producer can never
        // emit a record its own reader would refuse.
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
    for (name, start) in [("reserved", 8usize), ("reserved_mid", 24), ("reserved_tail", 44)] {
        if u32::from_le_bytes(payload[start..start + 4].try_into().expect("fixed slice")) != 0 {
            return Err(format!(
                "UNTRUSTED_CLOCK_RECOVERY {name} field is nonzero"
            ));
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

    /// The DELTA is the payload's reason for existing, and it is checked
    /// against the endpoints rather than believed. A producer that recorded a
    /// boolean "too far" and reconstructed a plausible number, or that
    /// understated the jump, writes a record its own reader refuses.
    ///
    /// Production change that makes this fail: dropping the
    /// `observed_delta != actual_delta` clause from
    /// `validate_untrusted_clock_recovery_payload`.
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

    /// This record may never stand in for an outage the producer could have
    /// named exactly. Below the ceiling the recovery was TRUSTED and owes an
    /// `H4CU` interval, which accounts for its hours; accepting it here would
    /// let a producer convert an accountable outage into an unaccountable one.
    ///
    /// Production change that makes this fail: dropping the
    /// `observed_delta <= trust_ceiling` clause.
    #[test]
    fn a_trusted_recovery_may_not_wear_this_records_name() {
        let mut inside = record();
        inside.resumed_hour_raw = hour(100 + 768);
        inside.observed_delta_hours = 768;
        let error = inside
            .encode()
            .expect_err("a delta at the ceiling must refuse");
        assert!(error.contains("does not exceed the trust ceiling"), "{error}");

        // And the ceiling itself cannot be manufactured downward to make an
        // ordinary jump look untrusted. THIS IS THE INTERESTING HALF: a producer
        // that could declare its own ceiling would set it to 1, call a two-hour
        // outage untrusted, and escape the `H4CU` branch that ACCOUNTS for the
        // hours -- turning a vouched loss into an unexplained one for free.
        //
        // MUTATION RUN: relax the ceiling clause back to `trust_ceiling <= 0`.
        // Observed failure:
        //   `a lowered ceiling must refuse: [72, 52, 67, 74, 1, 0, 48, 0, ...,
        //    100, 0, 0, 0, 0, 0, 0, 0, 99, 0, 0, 0, 0, 0, 0, 0, ...]`
        // -- a 100-hour jump encoding cleanly against a declared ceiling of 99.
        let mut lowered = record();
        lowered.last_observed_hour_raw = hour(100);
        lowered.resumed_hour_raw = hour(200);
        lowered.observed_delta_hours = 100;
        lowered.trust_ceiling_hours = 99;
        let error = lowered
            .encode()
            .expect_err("a lowered ceiling must refuse");
        assert!(error.contains("is not the pinned"), "{error}");

        let mut zero_ceiling = record();
        zero_ceiling.trust_ceiling_hours = 0;
        let error = zero_ceiling
            .encode()
            .expect_err("a zero ceiling must refuse");
        assert!(error.contains("is not the pinned"), "{error}");
    }

    /// An outage with no failed read is an ordinary hour jump. Letting one wear
    /// this record's name would excuse the exact hour-misattribution risk the
    /// record exists to disclose.
    ///
    /// Production change that makes this fail: dropping the
    /// `unreadable_events == 0` clause.
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

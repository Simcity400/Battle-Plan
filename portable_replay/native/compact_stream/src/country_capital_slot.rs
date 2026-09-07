use std::collections::BTreeSet;

pub const MAGIC: [u8; 4] = *b"CCS1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 48;
pub const MAX_GROUPS: usize = 65_536;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub override_raw: Option<i32>,
    pub base_raw: Option<i32>,
}

impl Operands {
    pub fn effective(self) -> Option<i32> {
        match self.override_raw {
            Some(0) => self.base_raw,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Failure {
    CountryIdentity = 1,
    OperandUnobtained = 2,
    GameStateUnreadable = 3,
    SlotHeaderUnreadable = 4,
    SlotUnreadable = 5,
    StateUnreadable = 6,
    Incoherent = 7,
    Bounds = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TemplateFailure {
    NullPointer = 1,
    Unreadable = 2,
    DatabaseUnavailable = 3,
    MembershipUnverified = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Absent,
    Failed(Failure),
    NonPositive,
    OutOfRange { slot_count: i32 },
    NullSlot { slot_count: i32 },
    Present { slot_count: i32, state_id: i32, template_id: i32 },
    TemplateUnavailable { slot_count: i32, state_id: i32, reason: TemplateFailure },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryGroup {
    pub tag_db_id: u32,
    pub operands: Operands,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapitalSlots {
    pub game_hour_raw: i32,
    pub observation_ordinal: u64,
    /// Missing rows carry no coverage or measured-empty claim.
    pub groups: Vec<CountryGroup>,
}

fn invalid(reason: &str) -> String {
    format!("country_capital_slot {reason}")
}

impl CountryGroup {
    pub fn validate(&self) -> Result<(), String> {
        if self.tag_db_id > i32::MAX as u32 {
            return Err(invalid("country identity exceeds signed engine range"));
        }
        let effective = self.operands.effective();
        match self.outcome {
            Outcome::Absent if self.operands != Operands::default() => {
                return Err(invalid("absent group carries operands"));
            }
            Outcome::Absent | Outcome::Failed(_) => return Ok(()),
            Outcome::NonPositive if effective.is_some_and(|v| v <= 0) => return Ok(()),
            Outcome::NonPositive => return Err(invalid("nonpositive outcome lacks nonpositive operand")),
            _ => {}
        }
        let Some(selected) = effective.filter(|v| *v > 0) else {
            return Err(invalid("slot outcome lacks positive effective operand"));
        };
        let count = match self.outcome {
            Outcome::OutOfRange { slot_count } => {
                if slot_count < 0 || selected < slot_count {
                    return Err(invalid("out-of-range outcome contradicts slot count"));
                }
                return Ok(());
            }
            Outcome::NullSlot { slot_count }
            | Outcome::TemplateUnavailable { slot_count, .. } => slot_count,
            Outcome::Present { slot_count, template_id, .. } => {
                if template_id < 0 {
                    return Err(invalid("present template ID is outside signed database range"));
                }
                slot_count
            }
            _ => unreachable!(),
        };
        if selected >= count {
            return Err(invalid("selected slot is outside witnessed count"));
        }
        Ok(())
    }
}

impl CapitalSlots {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        validate_groups(&self.groups)?;
        let mut out = vec![0; HEADER_LEN + self.groups.len() * ROW_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        out[8..10].copy_from_slice(&(ROW_LEN as u16).to_le_bytes());
        put(&mut out, 12, self.groups.len() as u32);
        out[16..20].copy_from_slice(&self.game_hour_raw.to_le_bytes());
        out[24..32].copy_from_slice(&self.observation_ordinal.to_le_bytes());
        for (group, row) in self.groups.iter().zip(out[HEADER_LEN..].chunks_exact_mut(ROW_LEN)) {
            put(row, 0, group.tag_db_id);
            for (at, value) in [(4, group.operands.override_raw), (12, group.operands.base_raw)] {
                if let Some(value) = value {
                    put(row, at, 1);
                    put(row, at + 4, value as u32);
                }
            }
            let (kind, count, state, template, reason) = match group.outcome {
                Outcome::Absent => (0, 0, 0, 0, 0),
                Outcome::Failed(reason) => (1, 0, 0, 0, reason as u32),
                Outcome::NonPositive => (2, 0, 0, 0, 0),
                Outcome::OutOfRange { slot_count } => (3, slot_count, 0, 0, 0),
                Outcome::NullSlot { slot_count } => (4, slot_count, 0, 0, 0),
                Outcome::Present { slot_count, state_id, template_id } => (5, slot_count, state_id, template_id, 0),
                Outcome::TemplateUnavailable { slot_count, state_id, reason } => (6, slot_count, state_id, 0, reason as u32),
            };
            for (at, value) in [(20, kind), (24, count as u32), (28, state as u32), (32, template as u32), (36, reason)] {
                put(row, at, value);
            }
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN {
            return Err(invalid("truncated header"));
        }
        if bytes[..4] != MAGIC || bytes[4..6] != VERSION.to_le_bytes()
            || bytes[6..8] != (HEADER_LEN as u16).to_le_bytes()
            || bytes[8..10] != (ROW_LEN as u16).to_le_bytes() || bytes[10..12] != [0, 0]
            || bytes[20..24] != [0; 4]
        {
            return Err(invalid("unsupported framing or reserved header"));
        }
        let count = word(bytes, 12) as usize;
        if count > MAX_GROUPS || bytes.len() != HEADER_LEN + count * ROW_LEN {
            return Err(invalid("bounded count or exact payload length mismatch"));
        }
        let mut groups = Vec::with_capacity(count);
        for row in bytes[HEADER_LEN..].chunks_exact(ROW_LEN) {
            if row[40..] != [0; 8] {
                return Err(invalid("reserved row bytes are nonzero"));
            }
            let operands = Operands { override_raw: operand(row, 4)?, base_raw: operand(row, 12)? };
            let (count, state, template, reason) = (word(row, 24) as i32, word(row, 28) as i32, word(row, 32) as i32, word(row, 36));
            let outcome = match word(row, 20) {
                0 if [count, state, template] == [0; 3] && reason == 0 => Outcome::Absent,
                1 if [count, state, template] == [0; 3] => Outcome::Failed(failure(reason)?),
                2 if [count, state, template] == [0; 3] && reason == 0 => Outcome::NonPositive,
                3 if state == 0 && template == 0 && reason == 0 => Outcome::OutOfRange { slot_count: count },
                4 if state == 0 && template == 0 && reason == 0 => Outcome::NullSlot { slot_count: count },
                5 if reason == 0 => Outcome::Present { slot_count: count, state_id: state, template_id: template },
                6 if template == 0 => Outcome::TemplateUnavailable { slot_count: count, state_id: state, reason: template_failure(reason)? },
                _ => return Err(invalid("unknown outcome or nonzero unused field")),
            };
            groups.push(CountryGroup { tag_db_id: word(row, 0), operands, outcome });
        }
        validate_groups(&groups)?;
        Ok(Self {
            game_hour_raw: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            observation_ordinal: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            groups,
        })
    }
}

fn validate_groups(groups: &[CountryGroup]) -> Result<(), String> {
    if groups.len() > MAX_GROUPS {
        return Err(invalid("too many country groups"));
    }
    let mut seen = BTreeSet::new();
    for group in groups {
        group.validate()?;
        if !seen.insert(group.tag_db_id) {
            return Err(invalid("duplicate country identity"));
        }
    }
    Ok(())
}

fn operand(row: &[u8], at: usize) -> Result<Option<i32>, String> {
    match (word(row, at), word(row, at + 4)) {
        (0, 0) => Ok(None),
        (1, value) => Ok(Some(value as i32)),
        _ => Err(invalid("unknown operand tag or nonzero absent operand")),
    }
}

fn failure(value: u32) -> Result<Failure, String> {
    Ok(match value {
        1 => Failure::CountryIdentity, 2 => Failure::OperandUnobtained,
        3 => Failure::GameStateUnreadable, 4 => Failure::SlotHeaderUnreadable,
        5 => Failure::SlotUnreadable, 6 => Failure::StateUnreadable,
        7 => Failure::Incoherent, 8 => Failure::Bounds,
        _ => return Err(invalid("unknown failure reason")),
    })
}

fn template_failure(value: u32) -> Result<TemplateFailure, String> {
    Ok(match value {
        1 => TemplateFailure::NullPointer, 2 => TemplateFailure::Unreadable,
        3 => TemplateFailure::DatabaseUnavailable, 4 => TemplateFailure::MembershipUnverified,
        _ => return Err(invalid("unknown template failure reason")),
    })
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn put(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(outcome: Outcome) -> CapitalSlots {
        CapitalSlots { game_hour_raw: -17, observation_ordinal: u64::MAX, groups: vec![CountryGroup {
            tag_db_id: 7, operands: Operands { override_raw: Some(4), base_raw: None }, outcome,
        }] }
    }

    #[test]
    fn roundtrip_preserves_independent_ids_signed_operands_and_custody() {
        let mut input = frame(Outcome::Present { slot_count: 9, state_id: 73, template_id: 21 });
        input.groups.push(CountryGroup { tag_db_id: 3, operands: Operands { override_raw: Some(i32::MIN), base_raw: Some(12) }, outcome: Outcome::NonPositive });
        let bytes = input.encode().unwrap();
        assert_eq!(CapitalSlots::decode(&bytes).unwrap(), input);
        assert_eq!(word(&bytes, HEADER_LEN + 8) as i32, 4);
        assert_eq!(word(&bytes, HEADER_LEN + 28) as i32, 73);
        assert_eq!(word(&bytes, HEADER_LEN + 32) as i32, 21);
    }

    #[test]
    fn absent_null_failed_and_template_unavailable_are_distinct() {
        let mut values = Vec::new();
        for outcome in [Outcome::Absent, Outcome::Failed(Failure::Incoherent), Outcome::NullSlot { slot_count: 9 }, Outcome::TemplateUnavailable { slot_count: 9, state_id: 73, reason: TemplateFailure::NullPointer }] {
            let mut input = frame(outcome);
            if outcome == Outcome::Absent { input.groups[0].operands = Operands::default(); }
            let bytes = input.encode().unwrap();
            assert_eq!(CapitalSlots::decode(&bytes).unwrap(), input);
            assert!(!values.contains(&bytes));
            values.push(bytes);
        }
    }

    #[test]
    fn only_measured_zero_override_selects_base_and_outcomes_obey_range() {
        assert_eq!(Operands { override_raw: None, base_raw: Some(4) }.effective(), None);
        assert_eq!(Operands { override_raw: Some(-1), base_raw: None }.effective(), Some(-1));
        let mut input = frame(Outcome::NullSlot { slot_count: 9 });
        input.groups[0].operands = Operands { override_raw: Some(0), base_raw: None };
        assert!(input.encode().is_err());
        input.groups[0].operands.base_raw = Some(4);
        assert!(input.encode().is_ok());
        for count in [-1, 0, 4] {
            input.groups[0].outcome = Outcome::NullSlot { slot_count: count };
            assert!(input.encode().is_err());
        }
        input.groups[0].outcome = Outcome::OutOfRange { slot_count: 4 };
        assert!(input.encode().is_ok());
        input.groups[0].outcome = Outcome::OutOfRange { slot_count: 5 };
        assert!(input.encode().is_err());
    }

    #[test]
    fn truncation_trailing_reserved_unknown_and_unused_fields_are_rejected() {
        let bytes = frame(Outcome::NullSlot { slot_count: 9 }).encode().unwrap();
        for len in 0..bytes.len() { assert!(CapitalSlots::decode(&bytes[..len]).is_err(), "{len}"); }
        let mut extra = bytes.clone(); extra.push(0); assert!(CapitalSlots::decode(&extra).is_err());
        for at in [0, 4, 6, 8, 10, 20, 23, HEADER_LEN + 40, HEADER_LEN + 47] {
            let mut bad = bytes.clone(); bad[at] ^= 1; assert!(CapitalSlots::decode(&bad).is_err(), "{at}");
        }
        for (at, value) in [(4, 2), (12, 2), (16, 1), (20, 7), (28, 1), (32, 1), (36, 99)] {
            let mut bad = bytes.clone(); put(&mut bad, HEADER_LEN + at, value);
            assert!(CapitalSlots::decode(&bad).is_err(), "{at}");
        }
    }

    #[test]
    fn duplicate_identities_and_oversized_counts_fail_before_allocation() {
        let input = frame(Outcome::NullSlot { slot_count: 9 });
        let mut duplicate = input.clone(); duplicate.groups.push(input.groups[0]);
        assert!(duplicate.encode().is_err());
        duplicate.groups[1].tag_db_id = 8;
        let mut bytes = duplicate.encode().unwrap();
        put(&mut bytes, HEADER_LEN + ROW_LEN, 7);
        assert!(CapitalSlots::decode(&bytes).is_err());
        let mut bytes = input.encode().unwrap();
        put(&mut bytes, 12, u32::MAX);
        assert!(CapitalSlots::decode(&bytes).is_err());
        assert!(CapitalSlots::decode(&CapitalSlots { groups: vec![], ..input }.encode().unwrap()).unwrap().groups.is_empty());
    }
    #[test]
    fn every_bounded_failure_cause_survives_and_unknown_causes_fail() {
        let mut encoded = Vec::new();
        for reason in [Failure::CountryIdentity, Failure::OperandUnobtained, Failure::GameStateUnreadable,
            Failure::SlotHeaderUnreadable, Failure::SlotUnreadable, Failure::StateUnreadable,
            Failure::Incoherent, Failure::Bounds] {
            let input = frame(Outcome::Failed(reason));
            let bytes = input.encode().unwrap();
            assert_eq!(CapitalSlots::decode(&bytes).unwrap(), input);
            assert!(!encoded.contains(&bytes)); encoded.push(bytes);
        }
        for reason in [TemplateFailure::NullPointer, TemplateFailure::Unreadable,
            TemplateFailure::DatabaseUnavailable, TemplateFailure::MembershipUnverified] {
            let input = frame(Outcome::TemplateUnavailable { slot_count: 9, state_id: 73, reason });
            let bytes = input.encode().unwrap();
            assert_eq!(CapitalSlots::decode(&bytes).unwrap(), input);
            assert!(!encoded.contains(&bytes)); encoded.push(bytes);
        }
        for mut bytes in encoded {
            put(&mut bytes, HEADER_LEN + 36, 99);
            assert!(CapitalSlots::decode(&bytes).is_err());
        }
    }

    #[test]
    fn literal_wire_vector_pins_raw_hour_and_field_positions() {
        let mut expected = Vec::from(*b"CCS1");
        expected.extend_from_slice(&[1, 0, 32, 0, 48, 0, 0, 0]);
        expected.extend_from_slice(&1_u32.to_le_bytes());
        expected.extend_from_slice(&(-17_i32).to_le_bytes());
        expected.extend_from_slice(&[0; 4]);
        expected.extend_from_slice(&u64::MAX.to_le_bytes());
        for word in [7_u32, 1, 4, 0, 0, 5, 9, 73, 21, 0, 0, 0] {
            expected.extend_from_slice(&word.to_le_bytes());
        }
        let input = frame(Outcome::Present { slot_count: 9, state_id: 73, template_id: 21 });
        assert_eq!(input.encode().unwrap(), expected);
        assert_eq!(CapitalSlots::decode(&expected).unwrap(), input);
        put(&mut expected, HEADER_LEN + 32, (-1_i32) as u32);
        assert!(CapitalSlots::decode(&expected).is_err());
        put(&mut expected, HEADER_LEN + 32, 21);
        put(&mut expected, HEADER_LEN, u32::MAX);
        assert!(CapitalSlots::decode(&expected).is_err());
    }

}

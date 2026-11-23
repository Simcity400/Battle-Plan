use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitCombatReference {
    pub combat: u64,
    pub kind: u8,
}

impl UnitCombatReference {
    fn check(&self) -> io::Result<()> {
        if self.combat == 0 || self.kind > 2 {
            return Err(invalid("invalid unit combat reference"));
        }
        Ok(())
    }
}

impl Value for UnitCombatReference {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 9;
    #[cfg(any(test, feature = "test-support"))]
    const STRUCT_FIELDS: usize = 2;

    fn encoded_bytes(&self) -> io::Result<usize> {
        self.check()?;
        Ok(Self::MINIMUM_ENCODED_BYTES)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.check()?;
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.check()?;
        Value::encode_reserved(&self.combat, output)?;
        Value::encode_reserved(&self.kind, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let value = Self {
            combat: Value::decode(input)?,
            kind: Value::decode(input)?,
        };
        value.check()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitCombatReferences {
    pub sample_hour: PacketHour,
    pub entries: Vec<UnitCombatReference>,
}
value_struct!(UnitCombatReferences {
    sample_hour: PacketHour,
    entries: Vec<UnitCombatReference>
});
impl OpaqueValue for UnitCombatReferences {}

pub fn check_unit_combat_references(
    field: &Recorded<Option<UnitCombatReferences>>,
    count: u32,
) -> io::Result<()> {
    if let Recorded::Value(Some(packet)) = field {
        if packet.entries.len() != count as usize {
            return Err(invalid("unit combat reference count differs"));
        }
        for entry in &packet.entries {
            entry.check()?;
        }
    }
    Ok(())
}

impl UnitCombatReferences {
    pub fn valid_at(&self, hour: i32, count: u32) -> bool {
        self.sample_hour.fresh_at(hour)
            && self.entries.len() == count as usize
            && self.entries.iter().all(|entry| entry.check().is_ok())
    }
}

replay_rows::wire_value!(
    UnitCombatReference,
);

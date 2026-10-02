use super::*;

/// Stable recording values, independent of the supported executable's addresses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EquipmentDestructionCause {
    #[default]
    Unknown = 0,
    LandCombat = 1,
    NavalTransport = 2,
    Immediate = 3,
    Attrition = 4,
    Garrison = 5,
    AirNuclear = 6,
    AirAccident = 7,
    AirStrategic = 8,
    AirMissionAa = 9,
    AirLandCombatCas = 10,
    AirRaid = 11,
    AirNavalAa = 12,
}

impl EquipmentDestructionCause {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::LandCombat => "land_combat",
            Self::NavalTransport => "naval_transport",
            Self::Immediate => "immediate",
            Self::Attrition => "attrition",
            Self::Garrison => "garrison",
            Self::AirNuclear => "air_nuclear",
            Self::AirAccident => "air_accident",
            Self::AirStrategic => "air_strategic",
            Self::AirMissionAa => "air_mission_aa",
            Self::AirLandCombatCas => "air_land_combat_cas",
            Self::AirRaid => "air_raid",
            Self::AirNavalAa => "air_naval_aa",
        }
    }
}

impl TryFrom<u8> for EquipmentDestructionCause {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Ok(match value {
            0 => Self::Unknown,
            1 => Self::LandCombat,
            2 => Self::NavalTransport,
            3 => Self::Immediate,
            4 => Self::Attrition,
            5 => Self::Garrison,
            6 => Self::AirNuclear,
            7 => Self::AirAccident,
            8 => Self::AirStrategic,
            9 => Self::AirMissionAa,
            10 => Self::AirLandCombatCas,
            11 => Self::AirRaid,
            12 => Self::AirNavalAa,
            _ => return Err(invalid("unsupported equipment destruction cause")),
        })
    }
}

impl From<engine::equipment_destruction::Cause> for EquipmentDestructionCause {
    fn from(cause: engine::equipment_destruction::Cause) -> Self {
        use engine::equipment_destruction::Cause;
        match cause {
            Cause::Unknown => Self::Unknown,
            Cause::LandCombat => Self::LandCombat,
            Cause::NavalTransport => Self::NavalTransport,
            Cause::Immediate => Self::Immediate,
            Cause::Attrition => Self::Attrition,
            Cause::Garrison => Self::Garrison,
            Cause::AirNuclear => Self::AirNuclear,
            Cause::AirAccident => Self::AirAccident,
            Cause::AirStrategic => Self::AirStrategic,
            Cause::AirMissionAa => Self::AirMissionAa,
            Cause::AirLandCombatCas => Self::AirLandCombatCas,
            Cause::AirRaid => Self::AirRaid,
            Cause::AirNavalAa => Self::AirNavalAa,
        }
    }
}

/// Actual equipment removed since the preceding consecutive hourly drain,
/// dated by this frame's drain read. Only positive (country,cause,design) rows.
/// `cause` is a stable value; engine::equipment_destruction owns the executable
/// caller map and evidence. An unrecognized caller records Unknown, counted in
/// collector status, without guessing a known cause or discarding its removals.
/// Variant identity is the same full handle as VariantDefinitions.id.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EquipmentDestructionRow {
    pub country: u32,
    pub cause: EquipmentDestructionCause,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(EquipmentDestructionRow, 104,
|row| (row.country as u128) << 72 | (row.cause as u128) << 64 | row.variant as u128,
|key| Self { country: (key >> 72) as u32, cause: EquipmentDestructionCause::try_from((key >> 64) as u8)?, variant: key as u64, amount_raw: 0 },
|key, input| Ok(Self { country: (key >> 72) as u32, cause: EquipmentDestructionCause::try_from((key >> 64) as u8)?,
    variant: key as u64, amount_raw: read_value(Kind::I64, input)? }),
{ 0 => (amount_raw): I64 }, |row| {
    if row.amount_raw <= 0 { Err(invalid("equipment destruction amount must be positive")) }
    else { Ok(()) }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_causes_roundtrip_and_unknown_is_explicit() {
        for value in 0..=12 {
            let cause = EquipmentDestructionCause::try_from(value).unwrap();
            assert_eq!(cause as u8, value);
            let row = EquipmentDestructionRow {
                country: 4999,
                cause,
                variant: 0xf00100000046,
                amount_raw: 7,
            };
            let delta = row.delta(None).unwrap().unwrap();
            assert_eq!(delta.apply::<EquipmentDestructionRow>(None).unwrap(), row);
            let mut bytes = Vec::new();
            row.encode_fields(&mut bytes).unwrap();
            assert_eq!(
                EquipmentDestructionRow::decode_fields(row.key(), &mut Decoder::new(&bytes))
                    .unwrap(),
                row
            );
        }
        assert_eq!(
            EquipmentDestructionCause::from(engine::equipment_destruction::Cause::Unknown),
            EquipmentDestructionCause::Unknown
        );
        for value in [13, 255] {
            assert!(EquipmentDestructionCause::try_from(value).is_err());
            let key = (4999_u128 << 72) | ((value as u128) << 64) | 0xf00100000046;
            let row = EquipmentDestructionRow {
                amount_raw: 7,
                ..Default::default()
            };
            let mut delta = row.delta(None).unwrap().unwrap();
            delta.key = key;
            assert!(delta.apply::<EquipmentDestructionRow>(None).is_err());
            let mut bytes = Vec::new();
            row.encode_fields(&mut bytes).unwrap();
            assert!(
                EquipmentDestructionRow::decode_fields(key, &mut Decoder::new(&bytes)).is_err()
            );
        }
    }
}

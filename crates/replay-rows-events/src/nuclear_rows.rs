use crate::event_feed_rows::EventFeedCountry;
use replay_rows::{
    codec::{Decoder, invalid_static as invalid},
    value::{Value, value_struct},
};
use std::io;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum NuclearStrikePart {
    #[default]
    WarSupport = 0,
    Buildings = 1,
    Units = 2,
    Equipment = 3,
}

impl Value for NuclearStrikePart {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(1)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        (*self as u8).encode_reserved(output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match u8::decode(input)? {
            0 => Ok(Self::WarSupport),
            1 => Ok(Self::Buildings),
            2 => Ok(Self::Units),
            3 => Ok(Self::Equipment),
            _ => Err(invalid("unknown nuclear strike part")),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearWarSupport {
    pub country: u32,
    pub before_raw: i64,
    pub after_raw: i64,
}
value_struct!(NuclearWarSupport {
    country: u32,
    before_raw: i64,
    after_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearBuildingDamage {
    pub state: u32,
    pub province: Option<u32>,
    pub controller: u32,
    pub building_type: u32,
    pub amount_raw: i64,
}
value_struct!(NuclearBuildingDamage {
    state: u32,
    province: Option<u32>,
    controller: u32,
    building_type: u32,
    amount_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearUnitDamage {
    pub handle: u64,
    pub class: u32,
    pub owner: u32,
    pub strength_before: i64,
    pub strength_after: i64,
    pub organisation_before: Option<i64>,
    pub organisation_after: Option<i64>,
    pub manpower_before: i64,
    pub manpower_after: i64,
}
value_struct!(NuclearUnitDamage {
    handle: u64,
    class: u32,
    owner: u32,
    strength_before: i64,
    strength_after: i64,
    organisation_before: Option<i64>,
    organisation_after: Option<i64>,
    manpower_before: i64,
    manpower_after: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearEquipmentLoss {
    pub country: u32,
    pub wing: bool,
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(NuclearEquipmentLoss {
    country: u32,
    wing: bool,
    variant: u64,
    amount_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearWingLoss {
    pub country: u32,
    pub manpower_before: i64,
    pub manpower_after: i64,
}
value_struct!(NuclearWingLoss {
    country: u32,
    manpower_before: i64,
    manpower_after: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearStrikeRefusal {
    pub part: NuclearStrikePart,
    pub reason: Vec<u8>,
    pub count: u32,
}
value_struct!(NuclearStrikeRefusal {
    part: NuclearStrikePart,
    reason: Vec<u8>,
    count: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NuclearStrikeRow {
    pub id: u64,
    pub hour: i32,
    pub launcher: EventFeedCountry,
    pub nuke_type: u32,
    pub province: u32,
    pub state: u32,
    pub owner: EventFeedCountry,
    pub controller: u32,
    pub war_support: Vec<NuclearWarSupport>,
    pub buildings: Vec<NuclearBuildingDamage>,
    pub units: Vec<NuclearUnitDamage>,
    pub equipment: Vec<NuclearEquipmentLoss>,
    pub refusals: Vec<NuclearStrikeRefusal>,
    pub wings: Vec<NuclearWingLoss>,
}

impl NuclearStrikeRow {
    pub fn complete(&self, part: NuclearStrikePart) -> bool {
        self.refusals.iter().all(|refusal| refusal.part != part)
    }
}

replay_rows::wire_value!(
    NuclearStrikePart,
);

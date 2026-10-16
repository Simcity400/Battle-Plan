//! Nuclear strikes as impact events ([`engine::nuclear_strike_sites`]), not
//! stockpile snapshots. A strike appears once, in the hourly drain after its
//! impact; `hour` is the impact-time clock read. Outcomes are what the
//! impact did before the authored `on_nuke_drop` effects ran.
use crate::{
    codec::{Decoder, invalid_static as invalid},
    event_feed_rows::EventFeedCountry,
    value::{Value, value_struct},
};
use std::io;

/// An outcome a strike row can lack, named by its refusals.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
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

/// One war-support adjustment call: the country's raw signed fixed5 war support before and after
/// its clamp.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// One building whose healthy levels the strike removed, located and typed as
/// [`crate::rows::BuildingBombingDamageRow`] is: `controller` is the location controller,
/// `province` is set for a province building, and `amount_raw` is pre-minus-post health
/// (a full level is 10,000,000). Buildings it left unchanged are not listed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// One unit the strike's immediate-damage calls reached, with the fields they write
/// before and after. A unit reached in several provinces of a state's strike appears once
/// per call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NuclearUnitDamage {
    pub handle: u64,
    /// The native class word: 0 a division, 13 a railway gun.
    pub class: u32,
    pub owner: u32,
    pub strength_before: i64,
    pub strength_after: i64,
    /// None for a railway gun, which has no organisation.
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

/// The strike's signed fixed5 pool difference of one variant for one country, summed over
/// the division pools changed by active damage or, for `wing`, the air wing pools changed
/// by aircraft removal. A negative amount is a native gain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// One air wing's crews removed with its planes: the wing's acquired manpower before and after.
/// Part of the equipment outcome, whose writer call it is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// A part of the outcome that was not observed, with how many times.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// One nuclear impact. `id` increases through the recording. A complete empty table is an
/// hour without strikes; an unavailable one is unknown. A part named in `refusals` is
/// incomplete; any other part lists everything the impact did to it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NuclearStrikeRow {
    pub id: u64,
    pub hour: i32,
    /// The launcher's own reference (the nuclear strike's country) and full native ID at the
    /// impact.
    pub launcher: EventFeedCountry,
    /// Native type ordinal (the nuclear strike's type); 1 strikes every province of the state.
    pub nuke_type: u32,
    pub province: u32,
    /// 0 for a province without a state.
    pub state: u32,
    /// The state owner, which the war support loss falls on, resolved at the
    /// impact; the reserved country (reference 0) without a state.
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
    /// Whether every outcome of `part` is listed.
    pub fn complete(&self, part: NuclearStrikePart) -> bool {
        self.refusals.iter().all(|refusal| refusal.part != part)
    }
}

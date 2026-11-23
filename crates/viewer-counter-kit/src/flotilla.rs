use crate::type_tint;
use std::{collections::BTreeSet, sync::Arc};
use viewer_foundation::allegiance::Allegiance;
use viewer_unit_reads::{counter_inputs, counter_order};

#[derive(Clone)]
pub struct Flotilla {
    pub members: Arc<[u64]>,
    pub source: u64,
    pub owner: u32,
    pub logical: Option<u32>,
    pub type_tint: Option<type_tint::Side>,
    pub side: Result<Allegiance, String>,
    pub flag_owner: Option<u32>,
    pub port: bool,
    pub coarse: bool,
    pub ships: Option<usize>,
    pub hidden: bool,
    pub visible_member: bool,
    pub cells: Option<NavalCells>,
    pub bars: NavalBars,
    pub supply: counter_inputs::NavalSupply,
    pub status: counter_inputs::NavalStatus,
    pub mission: Option<u32>,
    pub marker: Result<Option<[f32; 4]>, String>,
    pub moving: bool,
    pub item: Option<ItemIntel>,
    pub port_tooltip: Option<viewer_native_gui::counter_hovers::task_force_port::Metadata>,
    pub ideology: Result<Option<i32>, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemIntel {
    pub byte: u16,
    pub first: u64,
    pub ships: Option<usize>,
}

pub const MISSION_ITEM_DETAIL: u16 = 150;

impl Flotilla {
    pub fn no_intel_type(&self) -> bool {
        match self.item {
            Some(item) => item.byte < MISSION_ITEM_DETAIL,
            None => self.hidden,
        }
    }

    pub fn shows_pride(&self) -> Option<bool> {
        if self
            .item
            .is_some_and(|item| item.byte < MISSION_ITEM_DETAIL)
        {
            return Some(false);
        }
        self.status.pride
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NavalBars {
    Hidden,
    Fill(f32, f32),
    Unknown(String),
}

#[derive(Clone)]
pub struct NavalCells {
    pub capitals: Option<usize>,
    pub hull: counter_order::HullIcon,
    pub carrier: BTreeSet<Option<(&'static str, u32)>>,
}

pub const BARS_UNRECORDED: &str = "the strength and organisation bars are undecided: a \
     member's ratios (the ship counter operands) are not recorded";

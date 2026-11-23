use crate::gun_members::{GunFields, GunMember};
use std::collections::BTreeSet;
use viewer_unit_reads::counter_move;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GunPlaced {
    pub member: GunMember,
    pub route: GunRoute,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GunRoute {
    pub combat_role: u8,
    pub combat_count: u32,
    pub combat_province: u32,
    pub target: u32,
    pub naval_transfer: bool,
    pub ordinary_path: Vec<u32>,
    pub alternate_path: Vec<u32>,
}

impl GunRoute {
    pub fn of(fields: &GunFields<'_>) -> Self {
        Self {
            combat_role: fields.combat_role,
            combat_count: fields.operands.combat_count,
            combat_province: fields.combat_province,
            target: fields.operands.target,
            naval_transfer: fields.operands.naval_transfer != 0,
            ordinary_path: fields.ordinary_path.to_vec(),
            alternate_path: fields.alternate_path.to_vec(),
        }
    }

    pub fn route(&self) -> counter_move::UnitRoute<'_> {
        counter_move::UnitRoute {
            class: viewer_runtime::unit_class::CounterClass::RailwayGun,
            combat_role: self.combat_role,
            combat_count: self.combat_count,
            combat_province: self.combat_province,
            target: self.target,
            naval_transfer: self.naval_transfer,
            path_total: self.ordinary_path.len() as u32,
            path: &self.ordinary_path,
            alternate_total: self.alternate_path.len() as u32,
            alternate_path: &self.alternate_path,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayeredGuns {
    pub complete: bool,
    pub recorded: usize,
    pub members: Vec<GunPlaced>,
    pub unnamed: usize,
    pub refused: BTreeSet<String>,
}

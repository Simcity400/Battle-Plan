use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectNames {
    pub terrains: BTreeSet<Vec<u8>>,
    pub terrains_gap: Option<String>,
    pub equipment: BTreeSet<Vec<u8>>,
    pub equipment_gap: Option<String>,
    pub equipment_upgrades: BTreeSet<Vec<u8>>,
    pub equipment_upgrades_gap: Option<String>,
    pub modifiers: BTreeSet<Vec<u8>>,
    pub modifiers_gap: Option<String>,
    pub early_modifiers: BTreeSet<Vec<u8>>,
    pub early_modifiers_gap: Option<String>,
}
